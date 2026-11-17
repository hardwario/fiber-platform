//! MQTT through the FIBER's broker (`docs/mqtt-api.md`), as the agent's [`Bus`].
//!
//! rumqttc's event loop runs in a task of its own and hands over what comes in through a
//! channel, so the agent's loop never cancels a read half way. A publish while the broker is down
//! is dropped, not kept: after the reconnect the agent sends every retained topic again.

use std::time::Duration;

use rumqttc::{AsyncClient, Event, LastWill, MqttOptions, Packet, QoS};
use tokio::sync::mpsc;

use super::run::{Bus, BusEvent, PREFIX};
use super::topics;

/// The broker, from the agent's side.
pub struct Mqtt {
    client: AsyncClient,
    events: mpsc::Receiver<BusEvent>,
}

impl Mqtt {
    /// Connects to `host:port` as `fiber-agent`, with the last will on `fiber/status`, and keeps
    /// reconnecting.
    pub fn connect(host: &str, port: u16) -> Mqtt {
        let mut options = MqttOptions::new("fiber-agent", host, port);
        options.set_keep_alive(Duration::from_secs(15));
        options.set_clean_session(true);
        options.set_last_will(LastWill::new(
            format!("{PREFIX}status"),
            topics::offline().to_string(),
            QoS::AtLeastOnce,
            true,
        ));
        let (client, mut eventloop) = AsyncClient::new(options, 256);
        let (tx, events) = mpsc::channel(64);
        let subscriber = client.clone();
        tokio::spawn(async move {
            loop {
                match eventloop.poll().await {
                    Ok(Event::Incoming(Packet::ConnAck(_))) => {
                        let _ = subscriber.try_subscribe(format!("{PREFIX}cmd"), QoS::AtLeastOnce);
                        if tx.send(BusEvent::Connected).await.is_err() {
                            return;
                        }
                    }
                    Ok(Event::Incoming(Packet::Publish(publish)))
                        if publish.topic == format!("{PREFIX}cmd") =>
                    {
                        if publish.retain {
                            // A retained request is cleared and not run (docs/mqtt-api.md).
                            if !publish.payload.is_empty() {
                                let _ = subscriber.try_publish(
                                    &publish.topic,
                                    QoS::AtLeastOnce,
                                    true,
                                    Vec::new(),
                                );
                            }
                            continue;
                        }
                        if tx
                            .send(BusEvent::Request(publish.payload.to_vec()))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    Ok(_) => {}
                    Err(error) => {
                        eprintln!("mqtt: {error}; trying again in 2 s");
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                }
            }
        });
        Mqtt { client, events }
    }
}

impl Bus for Mqtt {
    async fn publish(&mut self, topic: String, payload: String, retain: bool) {
        let _ = self.client.try_publish(topic, QoS::AtLeastOnce, retain, payload);
    }

    async fn next(&mut self) -> BusEvent {
        match self.events.recv().await {
            Some(event) => event,
            None => std::future::pending().await,
        }
    }
}
