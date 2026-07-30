use prometheus::{Gauge, GaugeVec, IntCounter, IntCounterVec, Opts, Registry};

pub(crate) struct Metrics {
    pub(crate) registry: Registry,
    pub(crate) robots_online: Gauge,
    pub(crate) robots_stale: Gauge,
    pub(crate) robots_offline: Gauge,
    pub(crate) messages_received: IntCounter,
    pub(crate) telemetry_received: IntCounter,
    pub(crate) commands_created: IntCounter,
    pub(crate) commands_completed: IntCounter,
    pub(crate) command_failures: IntCounter,
    pub(crate) sensor_events_received: IntCounterVec,
    pub(crate) mqtt_connection_status: Gauge,
    pub(crate) telemetry_lag_seconds: Gauge,
    pub(crate) robot_position_x_cm: GaugeVec,
    pub(crate) robot_position_y_cm: GaugeVec,
    pub(crate) robot_velocity_cm_s: GaugeVec,
    pub(crate) robot_direction_degrees: GaugeVec,
    pub(crate) http_requests: IntCounterVec,
}

impl Metrics {
    pub(crate) fn new() -> anyhow::Result<Self> {
        let registry = Registry::new();
        let robots_online = Gauge::new("robots_online", "Number of robots currently online")?;
        let robots_stale = Gauge::new("robots_stale", "Number of robots currently stalled")?;
        let robots_offline = Gauge::new("robots_offline", "Number of robots currently offline")?;
        let messages_received = IntCounter::new(
            "robot_messages_received_total",
            "MQTT robot messages received by the backend",
        )?;
        let telemetry_received = IntCounter::new(
            "robot_telemetry_received_total",
            "Telemetry messages received by the backend",
        )?;
        let commands_created =
            IntCounter::new("commands_created_total", "Commands created through the API")?;
        let commands_completed =
            IntCounter::new("commands_completed_total", "Commands completed by robots")?;
        let command_failures =
            IntCounter::new("command_failures_total", "Commands reported as failed")?;
        let sensor_events_received = IntCounterVec::new(
            Opts::new(
                "robot_sensor_events_total",
                "Sensor events received from robots",
            ),
            &["event_type", "priority", "robot_id"],
        )?;
        let mqtt_connection_status =
            Gauge::new("mqtt_connection_status", "Backend MQTT connection status")?;
        let telemetry_lag_seconds = Gauge::new(
            "telemetry_ingestion_lag_seconds",
            "Seconds between telemetry recording and ingestion",
        )?;
        let robot_position_x_cm = GaugeVec::new(
            Opts::new(
                "robot_position_x_cm",
                "Latest robot position on the X axis in centimeters",
            ),
            &["robot_id"],
        )?;
        let robot_position_y_cm = GaugeVec::new(
            Opts::new(
                "robot_position_y_cm",
                "Latest robot position on the Y axis in centimeters",
            ),
            &["robot_id"],
        )?;
        let robot_velocity_cm_s = GaugeVec::new(
            Opts::new(
                "robot_velocity_cm_s",
                "Latest robot velocity in centimeters per second",
            ),
            &["robot_id"],
        )?;
        let robot_direction_degrees = GaugeVec::new(
            Opts::new(
                "robot_direction_degrees",
                "Latest robot direction in degrees",
            ),
            &["robot_id"],
        )?;
        let http_requests = IntCounterVec::new(
            Opts::new("backend_http_requests_total", "Backend HTTP requests"),
            &["route"],
        )?;

        registry.register(Box::new(robots_online.clone()))?;
        registry.register(Box::new(robots_stale.clone()))?;
        registry.register(Box::new(robots_offline.clone()))?;
        registry.register(Box::new(messages_received.clone()))?;
        registry.register(Box::new(telemetry_received.clone()))?;
        registry.register(Box::new(commands_created.clone()))?;
        registry.register(Box::new(commands_completed.clone()))?;
        registry.register(Box::new(command_failures.clone()))?;
        registry.register(Box::new(sensor_events_received.clone()))?;
        registry.register(Box::new(mqtt_connection_status.clone()))?;
        registry.register(Box::new(telemetry_lag_seconds.clone()))?;
        registry.register(Box::new(robot_position_x_cm.clone()))?;
        registry.register(Box::new(robot_position_y_cm.clone()))?;
        registry.register(Box::new(robot_velocity_cm_s.clone()))?;
        registry.register(Box::new(robot_direction_degrees.clone()))?;
        registry.register(Box::new(http_requests.clone()))?;

        Ok(Self {
            registry,
            robots_online,
            robots_stale,
            robots_offline,
            messages_received,
            telemetry_received,
            commands_created,
            commands_completed,
            command_failures,
            sensor_events_received,
            mqtt_connection_status,
            telemetry_lag_seconds,
            robot_position_x_cm,
            robot_position_y_cm,
            robot_velocity_cm_s,
            robot_direction_degrees,
            http_requests,
        })
    }
}
