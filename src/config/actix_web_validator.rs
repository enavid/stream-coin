use std::error::Error;
use validator:: ValidationErrors;


pub fn validation_errors_to_json(err: &actix_web_validator::Error) -> serde_json::Value {
    if let Some(validation_errors) = err.source().and_then(|e| e.downcast_ref::<ValidationErrors>()) {
        let mut error_list = vec![];

        for (field, field_errors) in validation_errors.field_errors() {
            for error in field_errors {
                if let Some(message) = &error.message {
                    error_list.push(serde_json::json!({
                        "field": field,
                        "message": message.to_string()
                    }));
                }
            }
        }

        return serde_json::json!({
            "code": 400,
            "message": "Validation failed",
            "errors": error_list
        });
    }

    serde_json::json!({
        "code": 400,
        "message": "Invalid JSON format",
        "error": err.to_string()
    })
}
