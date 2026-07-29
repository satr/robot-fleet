use anyhow::Context;

pub fn parse_mqtt_url(url: &str) -> anyhow::Result<(String, u16)> {
    let value = url.strip_prefix("mqtt://").unwrap_or(url);
    let (host, port) = value
        .rsplit_once(':')
        .context("MQTT_URL must look like mqtt://host:port")?;
    Ok((host.to_string(), port.parse()?))
}
