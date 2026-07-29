use robot_fleet_common::types::TelemetryMessage;
use serde_json::Value;
use tracing::info;

#[derive(Clone)]
pub(crate) struct KafkaPublisher {
    brokers: String,
}

impl KafkaPublisher {
    pub(crate) fn new(brokers: String) -> Self {
        Self { brokers }
    }

    pub(crate) async fn publish(&self, topic: &str, key: &str, payload: &Value) {
        info!(
            kafka_brokers = %self.brokers,
            topic,
            key,
            payload = %payload,
            "kafka publish placeholder"
        );
    }

    pub(crate) async fn publish_telemetry(&self, message: &TelemetryMessage) -> anyhow::Result<()> {
        self.publish(
            "robot-telemetry",
            &message.robot_id,
            &serde_json::to_value(message)?,
        )
        .await;
        Ok(())
    }
}
