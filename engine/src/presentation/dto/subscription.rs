use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct SubscribeRequest {
    pub strategy_id: String,
    #[schema(value_type = Option<String>)]
    pub max_position_size: Option<Decimal>,
    pub confidence_threshold: Option<f64>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateSubscriptionRequest {
    pub active: bool,
    #[schema(value_type = Option<String>)]
    pub max_position_size: Option<Decimal>,
    pub confidence_threshold: Option<f64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SubscriptionResponse {
    pub id: i64,
    pub user_id: i32,
    pub strategy_id: String,
    pub active: bool,
    #[schema(value_type = Option<String>)]
    pub max_position_size: Option<Decimal>,
    pub confidence_threshold: Option<f64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SubscriptionListResponse {
    pub subscriptions: Vec<SubscriptionResponse>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscribe_request_with_all_fields_deserializes_correctly() {
        let json =
            r#"{"strategy_id":"spread-1","max_position_size":"500","confidence_threshold":0.9}"#;
        let req: SubscribeRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.strategy_id, "spread-1");
        assert_eq!(req.max_position_size, Some(Decimal::new(500, 0)));
        assert_eq!(req.confidence_threshold, Some(0.9));
    }

    #[test]
    fn subscribe_request_with_optional_fields_absent_uses_none() {
        let json = r#"{"strategy_id":"spread-1"}"#;
        let req: SubscribeRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.strategy_id, "spread-1");
        assert!(req.max_position_size.is_none());
        assert!(req.confidence_threshold.is_none());
    }

    #[test]
    fn update_request_active_flag_deserializes_correctly() {
        let json = r#"{"active":false,"max_position_size":null,"confidence_threshold":null}"#;
        let req: UpdateSubscriptionRequest = serde_json::from_str(json).unwrap();
        assert!(!req.active);
        assert!(req.max_position_size.is_none());
    }

    #[test]
    fn subscription_response_serializes_with_all_fields() {
        let resp = SubscriptionResponse {
            id: 7,
            user_id: 3,
            strategy_id: "rsi-2".to_string(),
            active: true,
            max_position_size: Some(Decimal::new(1000, 0)),
            confidence_threshold: Some(0.8),
            created_at: DateTime::from_timestamp(0, 0).unwrap(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"id\":7"));
        assert!(json.contains("\"strategy_id\":\"rsi-2\""));
    }
}
