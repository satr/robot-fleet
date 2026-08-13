use anyhow::Context;
use rumqttc::{MqttOptions, Transport};

pub fn mqtt_options(url: &str, client_id: &str) -> anyhow::Result<MqttOptions> {
    mqtt_options_with_credentials(url, client_id, None, None)
}

pub fn mqtt_options_with_credentials(
    url: &str,
    client_id: &str,
    username: Option<&str>,
    password: Option<&str>,
) -> anyhow::Result<MqttOptions> {
    let (scheme, value) = url
        .split_once("://")
        .context("MQTT_URL must include a scheme")?;
    let authority = value.split('/').next().unwrap_or(value);
    let (credentials, authority) = authority
        .rsplit_once('@')
        .map(|(credentials, authority)| (Some(credentials), authority))
        .unwrap_or((None, authority));
    let (host, port) = authority
        .rsplit_once(':')
        .map(|(host, port)| (host, port.parse().unwrap_or(default_port(scheme))))
        .unwrap_or((authority, default_port(scheme)));
    let mut options = MqttOptions::new(client_id, host, port);
    if let Some((username, password)) = credentials.and_then(|value| value.split_once(':')) {
        options.set_credentials(username, password);
    } else if let (Some(username), Some(password)) = (username, password) {
        options.set_credentials(username, password);
    }
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
