use prometheus::{Gauge, IntCounter, Registry};

#[derive(Clone)]
pub(crate) struct Metrics {
    pub(crate) registry: Registry,
    pub(crate) mqtt_connection_status: Gauge,
    pub(crate) telemetry_sent: IntCounter,
    pub(crate) commands_processed: IntCounter,
}

impl Metrics {
    pub(crate) fn new() -> anyhow::Result<Self> {
        let registry = Registry::new();
        let mqtt_connection_status =
            Gauge::new("mqtt_connection_status", "Simulator MQTT connection status")?;
        let telemetry_sent =
            IntCounter::new("robot_telemetry_sent_total", "Telemetry messages published")?;
        let commands_processed = IntCounter::new(
            "robot_commands_processed_total",
            "Unique commands processed",
        )?;
        registry.register(Box::new(mqtt_connection_status.clone()))?;
        registry.register(Box::new(telemetry_sent.clone()))?;
        registry.register(Box::new(commands_processed.clone()))?;
        Ok(Self {
            registry,
            mqtt_connection_status,
            telemetry_sent,
            commands_processed,
        })
    }
}
