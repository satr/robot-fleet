use anyhow::Context;
use rumqttc::{MqttOptions, Transport};

pub fn mqtt_options(url: &str, client_id: &str) -> anyhow::Result<MqttOptions> {
    let (scheme, value) = url
        .split_once("://")
        .context("MQTT_URL must include a scheme")?;
    let authority = value.split('/').next().unwrap_or(value);
    let (host, port) = authority
        .rsplit_once(':')
        .map(|(host, port)| (host, port.parse().unwrap_or(default_port(scheme))))
        .unwrap_or((authority, default_port(scheme)));
    let mut options = MqttOptions::new(client_id, host, port);
    match scheme {
        "mqtt" => {}
        "ws" => {
            options.set_transport(Transport::ws());
        }
        "wss" => {
            options.set_transport(Transport::wss_with_default_config());
        }
        _ => anyhow::bail!("unsupported MQTT_URL scheme: {scheme}"),
    }
    Ok(options)
}

fn default_port(scheme: &str) -> u16 {
    match scheme {
        "wss" => 443,
        "ws" => 80,
        _ => 1883,
    }
}
