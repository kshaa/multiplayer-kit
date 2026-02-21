//! Channel handling for server-side actors.

use super::InternalServerEvent;
use crate::utils::{ChannelMessage, MessageBuffer};
use dashmap::DashMap;
use multiplayer_kit_protocol::{ChannelId, UserContext};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Handle a single channel for the server actor.
///
/// This function:
/// 1. Reads the channel type identification message
/// 2. Tracks the channel for user connection status
/// 3. Reads and decodes messages, forwarding to the actor
/// 4. Cleans up on disconnect
pub(super) async fn handle_server_channel<T: UserContext, M: ChannelMessage>(
    mut channel: multiplayer_kit_server::ServerChannel,
    user: T,
    channel_id: ChannelId,
    event_tx: mpsc::Sender<InternalServerEvent<T, M>>,
    user_channels: Arc<DashMap<ChannelId, (T, M::Channel)>>,
    pending_users: Arc<DashMap<T::Id, (T, HashMap<M::Channel, ChannelId>)>>,
    connected_users: Arc<DashMap<T::Id, T>>,
    expected_channels: usize,
) {
    let user_id = user.id();
    tracing::debug!("Channel {:?} opened for user {:?}", channel_id, user_id);
    let mut buffer = MessageBuffer::new();

    let channel_type: M::Channel;
    'outer: loop {
        let Some(data) = channel.read().await else {
            return;
        };

        for result in buffer.push(&data) {
            match result {
                Ok(msg) if msg.len() == 1 => {
                    if let Some(ct) = M::channel_from_id(msg[0]) {
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

    user_channels.insert(channel_id, (user.clone(), channel_type));

    let user_ready = {
        let mut entry = pending_users
            .entry(user_id.clone())
            .or_insert_with(|| (user.clone(), HashMap::new()));
        entry.1.insert(channel_type, channel_id);

        if entry.1.len() >= expected_channels {
            drop(entry);
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
            .send(InternalServerEvent::UserReady(user.clone()))
            .await;
    }

    loop {
        let Some(data) = channel.read().await else {
            break;
        };

        for result in buffer.push(&data) {
            match result {
                Ok(msg) => match M::decode(channel_type, &msg) {
                    Ok(message) => {
                        let _ = event_tx
                            .send(InternalServerEvent::Message {
                                sender: user.clone(),
                                channel: channel_type,
                                message,
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

    user_channels.remove(&channel_id);

    let user_gone = if connected_users.remove(&user_id).is_some() {
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
            .send(InternalServerEvent::UserGone(user.clone()))
            .await;
    }
}
