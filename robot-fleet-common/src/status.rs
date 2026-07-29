use chrono::{DateTime, Utc};

pub fn robot_status_from_last_seen(
    now: DateTime<Utc>,
    last_seen_at: Option<DateTime<Utc>>,
) -> &'static str {
    let Some(last_seen_at) = last_seen_at else {
        return "offline";
    };
    let age_seconds = now.signed_duration_since(last_seen_at).num_seconds();
    if age_seconds <= 5 {
        "online"
    } else if age_seconds <= 15 {
        "stale"
    } else {
        "offline"
    }
}
