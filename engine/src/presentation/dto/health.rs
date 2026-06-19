use std::collections::HashMap;

use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, PartialEq, Debug, Clone, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ServiceStatus {
    Up,
    Down,
    Degraded,
    Unknown,
}

#[derive(Serialize, ToSchema)]
pub struct HealthStatus {
    pub name: &'static str,
    pub version: &'static str,
    pub status: ServiceStatus,
    pub checks: HashMap<String, ServiceStatus>,
}

/// Returns the worst `ServiceStatus` across all checks:
/// any `Down` → `Down`; any `Unknown` (with no `Down`) → `Degraded`; otherwise `Up`.
pub fn worst_status(checks: &HashMap<String, ServiceStatus>) -> ServiceStatus {
    if checks.values().any(|s| s == &ServiceStatus::Down) {
        ServiceStatus::Down
    } else if checks.values().any(|s| s == &ServiceStatus::Unknown) {
        ServiceStatus::Degraded
    } else {
        ServiceStatus::Up
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_status_up_serializes_to_lowercase_up() {
        let v = serde_json::to_value(&ServiceStatus::Up).unwrap();
        assert_eq!(v, "up");
    }

    #[test]
    fn service_status_down_serializes_to_lowercase_down() {
        let v = serde_json::to_value(&ServiceStatus::Down).unwrap();
        assert_eq!(v, "down");
    }

    #[test]
    fn service_status_degraded_serializes_to_lowercase_degraded() {
        let v = serde_json::to_value(&ServiceStatus::Degraded).unwrap();
        assert_eq!(v, "degraded");
    }

    #[test]
    fn service_status_unknown_serializes_to_lowercase_unknown() {
        let v = serde_json::to_value(&ServiceStatus::Unknown).unwrap();
        assert_eq!(v, "unknown");
    }

    #[test]
    fn health_status_is_worst_of_checks_down_wins() {
        let mut checks = HashMap::new();
        checks.insert("redis".to_string(), ServiceStatus::Down);
        assert_eq!(worst_status(&checks), ServiceStatus::Down);
    }

    #[test]
    fn health_status_is_worst_of_checks_unknown_gives_degraded() {
        let mut checks = HashMap::new();
        checks.insert("kafka".to_string(), ServiceStatus::Unknown);
        assert_eq!(worst_status(&checks), ServiceStatus::Degraded);
    }

    #[test]
    fn health_status_is_worst_of_checks_up_when_all_up() {
        let mut checks = HashMap::new();
        checks.insert("redis".to_string(), ServiceStatus::Up);
        assert_eq!(worst_status(&checks), ServiceStatus::Up);
    }

    #[test]
    fn health_status_down_overrides_unknown() {
        let mut checks = HashMap::new();
        checks.insert("redis".to_string(), ServiceStatus::Down);
        checks.insert("kafka".to_string(), ServiceStatus::Unknown);
        assert_eq!(worst_status(&checks), ServiceStatus::Down);
    }

    #[test]
    fn health_status_is_worst_of_checks_empty_gives_up() {
        let checks: HashMap<String, ServiceStatus> = HashMap::new();
        assert_eq!(worst_status(&checks), ServiceStatus::Up);
    }
}
