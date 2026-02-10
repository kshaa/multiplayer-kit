//! Channel handling for server-side typed actors.

use super::ServerInternalEvent;
use crate::framing::MessageBuffer;
use crate::typed::TypedProtocol;
use dashmap::DashMap;
use multiplayer_kit_protocol::{ChannelId, UserContext};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Handle a single channel for the typed server actor.
///
/// This function:
/// 1. Reads the channel type identification message
/// 2. Tracks the channel for user connection status
/// 3. Reads and decodes messages, forwarding to the actor
/// 4. Cleans up on disconnect
pub(super) async fn handle_typed_channel<T: UserContext, P: TypedProtocol>(
    mut channel: multiplayer_kit_server::ServerChannel,
    user: T,
    channel_id: ChannelId,
    event_tx: mpsc::Sender<ServerInternalEvent<T, P>>,
    user_channels: Arc<DashMap<ChannelId, (T, P::Channel)>>,
    pending_users: Arc<DashMap<T::Id, (T, HashMap<P::Channel, ChannelId>)>>,
    connected_users: Arc<DashMap<T::Id, T>>,
    expected_channels: usize,
) {
    let user_id = user.id();
    tracing::debug!("Channel {:?} opened for user {:?}", channel_id, user_id);
    let mut buffer = MessageBuffer::new();

    // First message identifies channel type
    let channel_type: P::Channel;
    'outer: loop {
        let Some(data) = channel.read().await else {
            return;
        };

        for result in buffer.push(&data) {
            match result {
                Ok(msg) if msg.len() == 1 => {
                    if let Some(ct) = P::channel_from_id(msg[0]) {
                        channel_type = ct;
                        tracing::info!(
                            "Channel {:?} identified as {:?} for {}",
                            channel_id,
                            channel_type,
                            user
                        );
                        break 'outer;
                    }
                    tracing::warn!("Unknown channel type: {}", msg[0]);
                    return;
                }
                Ok(_) => {
                    tracing::warn!("Invalid channel identification message");
                    return;
                }
                Err(e) => {
                    tracing::warn!("Framing error during channel identification: {}", e);
                    return;
                }
            }
        }
    }

    // Register channel
    user_channels.insert(channel_id, (user.clone(), channel_type));

    // Track for user connection status
    let user_ready = {
        let mut entry = pending_users
            .entry(user_id.clone())
            .or_insert_with(|| (user.clone(), HashMap::new()));
        entry.1.insert(channel_type, channel_id);

        if entry.1.len() >= expected_channels {
            drop(entry); // Release DashMap ref before remove
            let (_, (user, _)) = pending_users.remove(&user_id).unwrap();
            connected_users.insert(user_id.clone(), user.clone());
            Some(user)
        } else {
            None
        }
    };

    if let Some(ref user) = user_ready {
        tracing::info!(
            "{} fully connected ({} channels established)",
            user,
            expected_channels
        );
        let _ = event_tx
            .send(ServerInternalEvent::UserReady(user.clone()))
            .await;
    }

    // Read messages
    loop {
        let Some(data) = channel.read().await else {
            break;
        };

        for result in buffer.push(&data) {
            match result {
                Ok(msg) => match P::decode(channel_type, &msg) {
                    Ok(event) => {
                        let _ = event_tx
                            .send(ServerInternalEvent::Message {
                                sender: user.clone(),
                                channel_type,
                                event,
                            })
                            .await;
                    }
                    Err(e) => {
                        tracing::warn!("Decode error: {}", e);
                    }
                },
                Err(e) => {
                    tracing::warn!("Framing error: {}", e);
                    break;
                }
            }
        }
    }

    // Channel closed - remove from tracking
    user_channels.remove(&channel_id);

    // Check if user is now disconnected
    let user_gone = if connected_users.remove(&user_id).is_some() {
        // Remove all remaining channels for this user
        let to_remove: Vec<_> = user_channels
            .iter()
            .filter(|r| r.value().0.id() == user_id)
            .map(|r| *r.key())
            .collect();
        for id in to_remove {
            user_channels.remove(&id);
        }
        Some(user.clone())
    } else {
        pending_users.remove(&user_id);
        None
    };

    if let Some(ref user) = user_gone {
        tracing::info!(
            "{} disconnected (channel {:?} closed)",
            user,
            channel_type
        );
        let _ = event_tx
            .send(ServerInternalEvent::UserGone(user.clone()))
            .await;
    }
}
