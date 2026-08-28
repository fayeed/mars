use mars_core::protocol::{
    Command, CommandRequest, CommandResponse, CoreEvent, EventPayload, ProtocolError,
    ResponsePayload,
};
use mars_core::{Core, ProtocolVersion};
use serde_json::{Value, json};

#[test]
fn command_request_round_trips_with_a_typed_payload() {
    let request = CommandRequest {
        protocol_version: ProtocolVersion::new(0, 1, 0),
        correlation_id: "req-0198".into(),
        command: Command::CoreInfo {},
    };

    let encoded = serde_json::to_value(&request).expect("request should serialize");

    assert_eq!(
        encoded["protocolVersion"],
        json!({ "major": 0, "minor": 1, "patch": 0 })
    );
    assert_eq!(encoded["correlationId"], "req-0198");
    assert_eq!(encoded["command"], json!({ "type": "coreInfo" }));
    assert_eq!(
        serde_json::from_value::<CommandRequest>(encoded).expect("request should deserialize"),
        request
    );
}

#[test]
fn command_response_round_trips_success_and_structured_error() {
    let success = CommandResponse::Success {
        protocol_version: ProtocolVersion::new(0, 1, 0),
        correlation_id: "req-0198".into(),
        result: ResponsePayload::CoreInfo {
            protocol_version: ProtocolVersion::new(0, 1, 0),
        },
    };
    let failure = CommandResponse::Failure {
        protocol_version: ProtocolVersion::new(0, 1, 0),
        correlation_id: "req-0199".into(),
        error: ProtocolError {
            code: "core.unavailable".into(),
            message: "Core is unavailable".into(),
            retryable: true,
            details: Some(json!({ "component": "storage" })),
        },
    };

    for response in [success, failure] {
        let encoded = serde_json::to_value(&response).expect("response should serialize");
        let decoded = serde_json::from_value::<CommandResponse>(encoded)
            .expect("response should deserialize");
        assert_eq!(decoded, response);
    }
}

#[test]
fn event_round_trip_preserves_unknown_payload_fields() {
    let wire = json!({
        "protocolVersion": { "major": 0, "minor": 1, "patch": 0 },
        "sequence": 7,
        "event": {
            "type": "indexProgress",
            "payload": { "filesSeen": 42 },
            "futureField": { "supportedLater": true }
        }
    });

    let event: CoreEvent = serde_json::from_value(wire.clone()).expect("event should deserialize");

    assert_eq!(event.event.event_type, "indexProgress");
    assert_eq!(event.event.payload, json!({ "filesSeen": 42 }));
    assert_eq!(
        event.event.extensions.get("futureField"),
        Some(&json!({ "supportedLater": true }))
    );
    assert_eq!(
        serde_json::to_value(event).expect("event should serialize"),
        wire
    );
}

#[test]
fn event_payload_constructor_rejects_reserved_extension_keys() {
    let result = EventPayload::new(
        "indexProgress",
        json!({ "filesSeen": 1 }),
        [("type".into(), Value::String("override".into()))].into(),
    );

    assert!(result.is_err());
}

#[test]
fn command_request_rejects_fields_outside_the_versioned_schema() {
    let wire = json!({
        "protocolVersion": { "major": 0, "minor": 1, "patch": 0 },
        "correlationId": "req-0198",
        "command": { "type": "coreInfo", "futureField": true }
    });

    assert!(serde_json::from_value::<CommandRequest>(wire).is_err());
}

#[test]
fn core_executes_a_versioned_request_and_preserves_correlation() {
    let response = Core::new().execute(CommandRequest {
        protocol_version: ProtocolVersion::new(0, 1, 0),
        correlation_id: "req-live".into(),
        command: Command::CoreInfo {},
    });

    assert!(matches!(
        response,
        CommandResponse::Success {
            correlation_id,
            result: ResponsePayload::CoreInfo {
                protocol_version: ProtocolVersion { major: 0, minor: 1, patch: 0 }
            },
            ..
        } if correlation_id == "req-live"
    ));
}

#[test]
fn core_rejects_an_incompatible_protocol_version() {
    let response = Core::new().execute(CommandRequest {
        protocol_version: ProtocolVersion::new(1, 0, 0),
        correlation_id: "req-newer".into(),
        command: Command::CoreInfo {},
    });

    assert!(matches!(
        response,
        CommandResponse::Failure { error, .. } if error.code == "protocol.version_mismatch"
    ));
}
