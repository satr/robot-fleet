use prometheus::{Gauge, IntCounter, IntCounterVec, Opts, Registry};

#[derive(Clone)]
pub(crate) struct Metrics {
    pub(crate) registry: Registry,
    pub(crate) mqtt_connection_status: Gauge,
    pub(crate) telemetry_sent: IntCounter,
    pub(crate) commands_processed: IntCounter,
    pub(crate) sensor_events_sent: IntCounterVec,
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
        let sensor_events_sent = IntCounterVec::new(
            Opts::new(
                "robot_simulator_sensor_events_sent_total",
                "Sensor events published by the simulator",
            ),
            &["event_type", "priority"],
        )?;
        registry.register(Box::new(mqtt_connection_status.clone()))?;
        registry.register(Box::new(telemetry_sent.clone()))?;
        registry.register(Box::new(commands_processed.clone()))?;
        registry.register(Box::new(sensor_events_sent.clone()))?;
        Ok(Self {
            registry,
            mqtt_connection_status,
            telemetry_sent,
            commands_processed,
            sensor_events_sent,
        })
    }
}
