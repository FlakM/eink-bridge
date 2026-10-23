use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::session::{Session, SessionOrigin, SessionStatus};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct CreateSessionRequest {
    #[serde(default)]
    pub schema_version: Option<u32>,
    #[serde(default)]
    pub title: Option<String>,
    pub content: String,
    #[serde(default)]
    pub callback_url: Option<String>,
    #[serde(default)]
    pub tags: HashMap<String, String>,
    #[serde(default)]
    pub starred: bool,
    #[serde(default)]
    pub origin: Option<SessionOrigin>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct SetStarredRequest {
    pub starred: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct SetCallbackRequest {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct SubmitReviewRequest {
    #[serde(default)]
    pub document_version: Option<u32>,
    #[serde(default)]
    pub schema_version: Option<u32>,
    #[serde(default)]
    pub typed_notes: String,
    #[serde(default)]
    pub verdict: Option<String>,
    #[serde(default)]
    pub annotations: Vec<AnnotationGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationGroup {
    pub anchor: Option<AnnotationAnchor>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub strokes: Vec<Vec<[f64; 2]>>,
    /// Parallel to `strokes`: `pressures[i][j]` is the 0..1 stylus pressure at `strokes[i][j]`.
    /// Empty or shorter-than-`strokes` means "no pressure data" and the OCR renderer falls back
    /// to a constant stroke width.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pressures: Vec<Vec<f64>>,
    #[serde(default)]
    pub recognized_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AnnotationAnchor {
    #[serde(rename = "lasso")]
    Lasso {
        elements: Vec<ElementRef>,
        lasso_bbox: BBox,
    },
    #[serde(rename = "proximity")]
    Proximity { elements: Vec<ElementRef> },
    #[serde(rename = "explicit")]
    Explicit { elements: Vec<ElementRef> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementRef {
    pub section_id: Option<String>,
    pub tag: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateSessionResponse {
    pub schema_version: u32,
    pub id: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSummaryResponse {
    pub schema_version: u32,
    pub id: String,
    pub title: Option<String>,
    pub status: &'static str,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tags: HashMap<String, String>,
    pub starred: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<SessionOrigin>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionDetailResponse {
    pub schema_version: u32,
    pub version: u32,
    pub id: String,
    pub title: Option<String>,
    pub status: &'static str,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub callback_url: Option<String>,
    pub tags: HashMap<String, String>,
    pub starred: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<SessionOrigin>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionResultResponse {
    pub schema_version: u32,
    pub id: String,
    pub title: Option<String>,
    pub status: &'static str,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub verdict: Option<&'static str>,
    pub typed_notes: Option<String>,
    pub annotation_images: Vec<String>,
    /// Fetchable URLs for the same images, in the same order. `annotation_images` are
    /// filesystem paths that only a client on this machine can open; a webhook receiver
    /// or a browser needs these.
    pub annotation_urls: Vec<String>,
    pub has_annotation: bool,
    pub review_duration_seconds: Option<i64>,
    pub tags: HashMap<String, String>,
    pub stroke_data: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<AnnotationGroup>,
    pub starred: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<SessionOrigin>,
}

impl SessionSummaryResponse {
    pub fn from_session(session: &Session) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            id: session.id.clone(),
            title: session.title.clone(),
            status: session.status.as_str(),
            created_at: session.created_at,
            updated_at: session.updated_at,
            tags: session.tags.clone(),
            starred: session.starred,
            origin: session.origin.clone(),
        }
    }
}

impl SessionDetailResponse {
    pub fn from_session(session: &Session) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            version: session.version,
            id: session.id.clone(),
            title: session.title.clone(),
            status: session.status.as_str(),
            created_at: session.created_at,
            updated_at: session.updated_at,
            callback_url: session.callback_url.clone(),
            tags: session.tags.clone(),
            starred: session.starred,
            origin: session.origin.clone(),
        }
    }
}

impl SessionResultResponse {
    pub fn from_session(session: &Session) -> Self {
        let review_duration_seconds = match session.status {
            SessionStatus::Submitted => Some(
                session
                    .updated_at
                    .signed_duration_since(session.created_at)
                    .num_seconds()
                    .max(0),
            ),
            _ => None,
        };

        Self {
            schema_version: SCHEMA_VERSION,
            id: session.id.clone(),
            title: session.title.clone(),
            status: session.status.as_str(),
            created_at: session.created_at,
            updated_at: session.updated_at,
            verdict: session.verdict.as_ref().map(|v| v.as_str()),
            typed_notes: session.typed_notes.clone(),
            annotation_images: session.annotation_images.clone(),
            annotation_urls: (0..session.annotation_images.len())
                .map(|index| format!("/api/sessions/{}/annotation/{index}", session.id))
                .collect(),
            // Any of the three carries ink. The rendered images are usually present, but
            // a submission whose images were never written still has the strokes.
            has_annotation: !session.annotation_images.is_empty()
                || !session.annotations.is_empty()
                || session.stroke_data.is_some(),
            review_duration_seconds,
            tags: session.tags.clone(),
            stroke_data: session.stroke_data.clone(),
            annotations: session.annotations.clone(),
            starred: session.starred,
            origin: session.origin.clone(),
        }
    }
}

pub fn openapi_spec() -> serde_json::Value {
    serde_json::json!({
        "openapi": "3.1.0",
        "info": {
            "title": "E-Ink Bridge API",
            "version": "1.0.0",
            "description": "AI-friendly review session API for pushing markdown to the e-ink device and collecting structured review results."
        },
        "paths": {
            "/api/health": {
                "get": {
                    "summary": "Health check",
                    "responses": {
                        "200": {"description": "Server healthy"}
                    }
                }
            },
            "/api/openapi.json": {
                "get": {
                    "summary": "OpenAPI document",
                    "responses": {
                        "200": {"description": "OpenAPI spec"}
                    }
                }
            },
            "/api/sessions": {
                "get": {
                    "summary": "List sessions",
                    "parameters": [
                        {
                            "name": "status",
                            "in": "query",
                            "required": false,
                            "schema": {"type": "string"},
                            "description": "Filter by session status (Active, Submitted, Cancelled, Expired)"
                        },
                        {
                            "name": "starred",
                            "in": "query",
                            "required": false,
                            "schema": {"type": "boolean"},
                            "description": "Filter to only starred (true) or only unstarred (false) sessions"
                        }
                    ],
                    "responses": {
                        "200": {
                            "description": "Session summaries (starred sessions sorted first)",
                            "content": {
                                "application/json": {
                                    "schema": {"type": "array", "items": {"$ref": "#/components/schemas/SessionSummaryResponse"}}
                                }
                            }
                        }
                    }
                },
                "post": {
                    "summary": "Create session",
                    "description": "Supports plain text markdown, a JSON body, or multipart/form-data carrying the images the document displays as repeated `image` parts. A document refers to an attached image as `image/<name>`; the renderer points that at the session's image route.",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": {"$ref": "#/components/schemas/CreateSessionRequest"}
                            },
                            "text/plain": {
                                "schema": {"type": "string"}
                            },
                            "multipart/form-data": {
                                "schema": {
                                    "type": "object",
                                    "properties": {
                                        "content": {"type": "string"},
                                        "title": {"type": "string"},
                                        "callback_url": {"type": "string"},
                                        "starred": {"type": "string", "description": "true or false"},
                                        "tags": {"type": "string", "description": "JSON object of string values"},
                                        "origin": {"type": "string", "description": "JSON SessionOrigin"},
                                        "image": {
                                            "type": "string",
                                            "format": "binary",
                                            "description": "Repeatable. The file name is the name the markdown references; png, jpg, jpeg, webp or gif."
                                        }
                                    }
                                }
                            }
                        }
                    },
                    "responses": {
                        "201": {
                            "description": "Session created",
                            "content": {
                                "application/json": {
                                    "schema": {"$ref": "#/components/schemas/CreateSessionResponse"}
                                }
                            }
                        }
                    }
                }
            },
            "/api/sessions/{id}": {
                "get": {
                    "summary": "Get session metadata",
                    "responses": {
                        "200": {
                            "description": "Session detail",
                            "content": {
                                "application/json": {
                                    "schema": {"$ref": "#/components/schemas/SessionDetailResponse"}
                                }
                            }
                        },
                        "404": {"description": "Session not found"}
                    }
                },
                "delete": {
                    "summary": "Cancel session",
                    "responses": {
                        "200": {"description": "Session cancelled"},
                        "404": {"description": "Session not found"}
                    }
                }
            },
            "/api/sessions/{id}/star": {
                "put": {
                    "summary": "Toggle starred flag on a session",
                    "description": "Starred sessions are pinned: they survive purge, expire_stale, and are reloaded from disk in any state.",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": {"$ref": "#/components/schemas/SetStarredRequest"}
                            }
                        }
                    },
                    "responses": {
                        "200": {"description": "Star flag updated"},
                        "404": {"description": "Session not found"}
                    }
                }
            },
            "/api/sessions/{id}/result": {
                "get": {
                    "summary": "Poll session result",
                    "responses": {
                        "200": {
                            "description": "Submitted review result",
                            "content": {
                                "application/json": {
                                    "schema": {"$ref": "#/components/schemas/SessionResultResponse"}
                                }
                            }
                        },
                        "204": {"description": "No change yet"},
                        "404": {"description": "Session not found"},
                        "410": {"description": "Session cancelled or expired"}
                    }
                }
            },
            "/api/sessions/{id}/feedback": {
                "get": {
                    "summary": "Recover durable pending feedback and latest content-update instructions",
                    "responses": {"200": {"description": "version, pending (annotation_result or null), last_content_update (version_updated or null)"}, "404": {"description": "Session not found"}}
                }
            },
            "/api/sessions/{id}/image/{name}": {
                "post": {
                    "summary": "Attach a picture to the document",
                    "description": "The raw image bytes. The name is one path segment ending in .png, .jpg, .jpeg, .webp or .gif, and the bytes must be that format. The document shows it as `image/<name>`, which the renderer resolves to this route. Only an active session accepts a new image.",
                    "parameters": [
                        {
                            "name": "name",
                            "in": "path",
                            "required": true,
                            "schema": {"type": "string"},
                            "description": "A plain file name, matching [A-Za-z0-9._-]+ with an image extension"
                        }
                    ],
                    "requestBody": {
                        "required": true,
                        "content": {
                            "image/png": {"schema": {"type": "string", "format": "binary"}},
                            "image/jpeg": {"schema": {"type": "string", "format": "binary"}},
                            "image/webp": {"schema": {"type": "string", "format": "binary"}},
                            "image/gif": {"schema": {"type": "string", "format": "binary"}}
                        }
                    },
                    "responses": {
                        "201": {"description": "Stored; returns {name, url, markdown}"},
                        "400": {"description": "Unusable name or bytes that are not the named format"},
                        "404": {"description": "Session not found"},
                        "409": {"description": "Session no longer active"}
                    }
                },
                "get": {
                    "summary": "Fetch a picture the document displays",
                    "responses": {
                        "200": {"description": "The image, with its content type, nosniff, and no-store"},
                        "404": {"description": "No such image in this session"}
                    }
                }
            },
            "/api/sessions/{id}/reanchor": {
                "post": {
                    "summary": "Resolve previous layout elements to current layout elements by text",
                    "description": "Submit previous/current arrays of {i,tag,text}, at most 1000 elements, 150 characters per text. Returns targets parallel to previous: current element indices, or null for missing/ambiguous matches. Client applies actual layout displacement.",
                    "responses": {"200": {"description": "targets array"}, "400": {"description": "Invalid or excessive layout"}, "404": {"description": "Session not found"}}
                }
            },
            "/api/sessions/{id}/content": {
                "get": {
                    "summary": "Recover current markdown and version to resume a review without scraping HTML",
                    "responses": {"200": {"description": "content, version, status"}, "404": {"description": "Session not found"}}
                },
                "put": {
                    "summary": "Rewrite content, optionally acknowledge a feedback request",
                    "description": "JSON {content,expected_version?,feedback_id?,consumed_annotations?} or plain markdown. consumed_annotations are zero-based indices of incorporated notes, require matching feedback_id, and resolve to consumed_strokes for native clients. Unacknowledged feedback survives manual edits.",
                    "responses": {"200": {"description": "New version"}, "400": {"description": "Invalid feedback ID or annotation index"}, "404": {"description": "Session not found"}, "409": {"description": "Stale version or terminal session"}}
                }
            },
            "/api/sessions/{id}/request_update": {
                "post": {
                    "summary": "Persist feedback before notifying agents",
                    "description": "Accepts SubmitReviewRequest. Pending feedback is replayed on WebSocket connection and exposed through GET /feedback until explicitly acknowledged by feedback_id.",
                    "responses": {"200": {"description": "Feedback durably saved; agent response not yet confirmed"}, "404": {"description": "Session not found"}, "409": {"description": "Document changed during recognition or session inactive"}}
                }
            },
            "/api/sessions/{id}/submit": {
                "post": {
                    "summary": "Submit review",
                    "description": "Supports multipart form submissions from the Android client and JSON submissions from AI/native clients.",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": {"$ref": "#/components/schemas/SubmitReviewRequest"}
                            },
                            "multipart/form-data": {
                                "schema": {
                                    "type": "object",
                                    "properties": {
                                        "typed_notes": {"type": "string"},
                                        "annotation": {"type": "string", "format": "binary"},
                                        "stroke_data": {
                                            "type": "string",
                                            "description": "JSON: {canvas_width, canvas_height, strokes:[[[x,y],...],...]}"
                                        },
                                        "annotations": {
                                            "type": "string",
                                            "description": "JSON array of AnnotationGroup objects"
                                        }
                                    }
                                }
                            }
                        }
                    },
                    "responses": {
                        "200": {"description": "Review submitted"},
                        "404": {"description": "Session not found"},
                        "409": {"description": "Session not active"}
                    }
                }
            }
        },
        "components": {
            "schemas": {
                "CreateSessionRequest": {
                    "type": "object",
                    "required": ["content"],
                    "properties": {
                        "schema_version": {"type": "integer"},
                        "title": {"type": ["string", "null"]},
                        "content": {"type": "string"},
                        "callback_url": {"type": ["string", "null"]},
                        "tags": {
                            "type": "object",
                            "additionalProperties": {"type": "string"}
                        },
                        "starred": {"type": "boolean"},
                        "origin": {"$ref": "#/components/schemas/SessionOrigin"}
                    }
                },
                "SetStarredRequest": {
                    "type": "object",
                    "required": ["starred"],
                    "properties": {
                        "starred": {"type": "boolean"}
                    }
                },
                "SessionOrigin": {
                    "type": "object",
                    "description": "Metadata about where the session originated (cwd, host, git state).",
                    "properties": {
                        "cwd": {"type": ["string", "null"]},
                        "host": {"type": ["string", "null"]},
                        "tool": {"type": ["string", "null"]},
                        "git_branch": {"type": ["string", "null"]},
                        "git_remote": {"type": ["string", "null"]}
                    }
                },
                "SubmitReviewRequest": {
                    "type": "object",
                    "properties": {
                        "document_version": {"type": "integer", "description": "Reject feedback from an outdated rendered document"},
                        "schema_version": {"type": "integer"},
                        "typed_notes": {"type": "string"},
                        "verdict": {"type": ["string", "null"]},
                        "annotations": {
                            "type": "array",
                            "items": {"$ref": "#/components/schemas/AnnotationGroup"}
                        }
                    }
                },
                "CreateSessionResponse": {
                    "type": "object",
                    "properties": {
                        "schema_version": {"type": "integer"},
                        "id": {"type": "string"},
                        "url": {"type": "string"}
                    }
                },
                "SessionSummaryResponse": {
                    "type": "object",
                    "properties": {
                        "schema_version": {"type": "integer"},
                        "id": {"type": "string"},
                        "title": {"type": ["string", "null"]},
                        "status": {"type": "string"},
                        "created_at": {"type": "string", "format": "date-time"},
                        "updated_at": {"type": "string", "format": "date-time"},
                        "tags": {"type": "object", "additionalProperties": {"type": "string"}},
                        "starred": {"type": "boolean"},
                        "origin": {"$ref": "#/components/schemas/SessionOrigin"}
                    }
                },
                "SessionDetailResponse": {
                    "allOf": [
                        {"$ref": "#/components/schemas/SessionSummaryResponse"},
                        {"type": "object", "properties": {"version": {"type": "integer"}, "callback_url": {"type": ["string", "null"]}}}
                    ]
                },
                "SessionResultResponse": {
                    "type": "object",
                    "properties": {
                        "schema_version": {"type": "integer"},
                        "id": {"type": "string"},
                        "title": {"type": ["string", "null"]},
                        "status": {"type": "string"},
                        "created_at": {"type": "string", "format": "date-time"},
                        "updated_at": {"type": "string", "format": "date-time"},
                        "verdict": {"type": ["string", "null"]},
                        "typed_notes": {"type": ["string", "null"]},
                        "annotation_images": {"type": "array", "items": {"type": "string"}},
                        "annotation_urls": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Fetchable URLs for the annotation images, in the same order as annotation_images"
                        },
                        "has_annotation": {"type": "boolean"},
                        "review_duration_seconds": {"type": ["integer", "null"]},
                        "tags": {"type": "object", "additionalProperties": {"type": "string"}},
                        "stroke_data": {
                            "description": "Raw pen strokes: {canvas_width, canvas_height, strokes:[[[x,y],...],...]}"
                        },
                        "annotations": {
                            "type": "array",
                            "items": {"$ref": "#/components/schemas/AnnotationGroup"}
                        },
                        "starred": {"type": "boolean"},
                        "origin": {"$ref": "#/components/schemas/SessionOrigin"}
                    }
                },
                "AnnotationGroup": {
                    "type": "object",
                    "description": "A group of strokes optionally anchored to document elements",
                    "properties": {
                        "anchor": {
                            "description": "Anchor binding strokes to document elements"
                        },
                        "strokes": {
                            "type": "array",
                            "description": "Array of strokes, each stroke is an array of [x,y] points"
                        },
                        "pressures": {
                            "type": "array",
                            "description": "Optional parallel array: per-point 0..1 stylus pressure for each stroke"
                        },
                        "recognized_text": {
                            "type": ["string", "null"]
                        },
                        "color": {
                            "type": ["string", "null"],
                            "description": "Hex color of the annotation stroke (e.g. #CC0000)"
                        }
                    }
                },
                "ElementRef": {
                    "type": "object",
                    "properties": {
                        "section_id": {"type": ["string", "null"]},
                        "tag": {"type": "string"},
                        "text": {"type": "string"}
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionManager;
    use serde_json::Value;
    use std::path::PathBuf;

    /// A session with every field populated, built through the real constructor so the
    /// defaults are the ones the server actually applies.
    fn populated_session() -> Session {
        let dir = tempfile::tempdir().unwrap();
        let mut manager = SessionManager::new(dir.path().to_path_buf());
        let mut session = manager.create(
            "# Review\n\nBody".into(),
            Some("A title".into()),
            Some("http://localhost:9999/hook".into()),
            HashMap::from([("repo".to_string(), "eink-bridge".to_string())]),
            true,
            Some(SessionOrigin {
                cwd: Some("/tmp".into()),
                host: Some("amd-pc".into()),
                tool: Some("eink-pr".into()),
                git_branch: Some("main".into()),
                git_remote: Some("github:FlakM/eink-bridge".into()),
            }),
        );
        session.status = SessionStatus::Submitted;
        session.typed_notes = Some("CHANGES: needs work".into());
        session.verdict = Some(crate::verdict::Verdict::Changes);
        session.annotation_images = vec!["/state/annotations/img_1.png".into()];
        session.stroke_data = Some(serde_json::json!({"canvas_width": 992, "canvas_height": 1400}));
        session.annotations = vec![AnnotationGroup {
            anchor: Some(AnnotationAnchor::Explicit {
                elements: vec![ElementRef {
                    section_id: Some("s-1".into()),
                    tag: "P".into(),
                    text: "Body".into(),
                }],
            }),
            strokes: vec![vec![[1.0, 2.0], [3.0, 4.0]]],
            pressures: vec![vec![0.5, 0.5]],
            recognized_text: Some("WRITE A TLDR".into()),
            color: Some("#CC0000".into()),
        }];
        session.updated_at = session.created_at + chrono::Duration::seconds(75);
        session
    }

    /// The properties a schema describes, following `allOf` and `$ref` one level deep,
    /// which is how the detail response is written.
    fn described_properties(spec: &Value, schema: &str) -> Vec<String> {
        let node = &spec["components"]["schemas"][schema];
        let mut names: Vec<String> = Vec::new();
        let collect = |node: &Value, names: &mut Vec<String>| {
            if let Some(props) = node.get("properties").and_then(Value::as_object) {
                names.extend(props.keys().cloned());
            }
        };
        collect(node, &mut names);
        for part in node
            .get("allOf")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            if let Some(reference) = part.get("$ref").and_then(Value::as_str) {
                let referenced = reference.rsplit('/').next().unwrap();
                collect(&spec["components"]["schemas"][referenced], &mut names);
            }
            collect(&part, &mut names);
        }
        names.sort();
        names.dedup();
        names
    }

    #[test]
    fn every_field_a_response_sends_is_described_in_the_spec() {
        // The spec is hand-written while the responses are serialised from structs, so
        // nothing but a test keeps the two together. A field the spec forgets is a field
        // the CLI and the app have no reason to read.
        let spec = openapi_spec();
        let session = populated_session();
        let cases: Vec<(&str, Value)> = vec![
            (
                "SessionSummaryResponse",
                serde_json::to_value(SessionSummaryResponse::from_session(&session)).unwrap(),
            ),
            (
                "SessionDetailResponse",
                serde_json::to_value(SessionDetailResponse::from_session(&session)).unwrap(),
            ),
            (
                "SessionResultResponse",
                serde_json::to_value(SessionResultResponse::from_session(&session)).unwrap(),
            ),
        ];
        for (schema, body) in cases {
            let described = described_properties(&spec, schema);
            assert!(
                !described.is_empty(),
                "{schema} describes nothing, so this check would be vacuous"
            );
            for field in body.as_object().expect("an object").keys() {
                assert!(
                    described.contains(field),
                    "{schema} sends '{field}' but the spec does not describe it: {described:?}"
                );
            }
        }
    }

    #[test]
    fn a_review_with_ink_reports_that_it_has_one() {
        // `has_annotation` is how a reader tells whether there is anything handwritten to
        // look at. Deriving it from the rendered images alone said "no" for a review that
        // carried strokes whose images were never rendered.
        let mut session = populated_session();
        session.annotations.clear();
        session.stroke_data = None;
        session.annotation_images.clear();
        assert!(!SessionResultResponse::from_session(&session).has_annotation);

        session.annotations = populated_session().annotations;
        assert!(
            SessionResultResponse::from_session(&session).has_annotation,
            "strokes without rendered images are still ink"
        );

        session.annotations.clear();
        session.stroke_data = Some(serde_json::json!({"strokes": []}));
        assert!(
            SessionResultResponse::from_session(&session).has_annotation,
            "raw stroke data is still ink"
        );

        session.stroke_data = None;
        session.annotation_images = vec!["/state/annotations/img_1.png".into()];
        assert!(SessionResultResponse::from_session(&session).has_annotation);
    }

    #[test]
    fn annotation_groups_round_trip_through_the_result() {
        // The app posts this shape and the result echoes it back; a field dropped in the
        // middle loses the pressure data or the anchor the note was bound to.
        let session = populated_session();
        let body = serde_json::to_value(SessionResultResponse::from_session(&session)).unwrap();
        let group = &body["annotations"][0];
        assert_eq!(group["color"], "#CC0000");
        assert_eq!(group["recognized_text"], "WRITE A TLDR");
        assert_eq!(group["strokes"][0][0][0], 1.0);
        assert_eq!(group["pressures"][0][1], 0.5);
        assert_eq!(group["anchor"]["type"], "explicit");
        assert_eq!(group["anchor"]["elements"][0]["tag"], "P");
    }

    #[test]
    fn a_submitted_review_reports_its_duration_and_a_live_one_does_not() {
        let session = populated_session();
        let result = SessionResultResponse::from_session(&session);
        assert_eq!(result.review_duration_seconds, Some(75));
        assert_eq!(result.verdict, Some("changes"));
        // The display form is capitalised and every client reads it that way
        // (SessionAdapter.kt, eink-channel.ts); the query filter is case-insensitive.
        assert_eq!(result.status, "Submitted");

        let mut active = session.clone();
        active.status = SessionStatus::Active;
        assert_eq!(
            SessionResultResponse::from_session(&active).review_duration_seconds,
            None,
            "a review still being written has no duration"
        );
    }

    #[test]
    fn a_review_edited_before_submitting_never_reports_negative_time() {
        // Clocks move: an update can land with an earlier timestamp than the create.
        let mut session = populated_session();
        session.updated_at = session.created_at - chrono::Duration::seconds(30);
        assert_eq!(
            SessionResultResponse::from_session(&session).review_duration_seconds,
            Some(0)
        );
    }

    #[test]
    fn the_summary_and_detail_agree_with_the_session_they_describe() {
        let session = populated_session();
        let summary = SessionSummaryResponse::from_session(&session);
        assert_eq!(summary.id, session.id);
        assert_eq!(summary.title.as_deref(), Some("A title"));
        assert_eq!(summary.status, "Submitted");
        assert!(summary.starred, "a pinned review stays pinned");
        assert_eq!(summary.schema_version, SCHEMA_VERSION);

        let detail = SessionDetailResponse::from_session(&session);
        assert_eq!(detail.version, session.version);
        assert_eq!(
            detail.callback_url.as_deref(),
            Some("http://localhost:9999/hook")
        );
        assert_eq!(
            detail.tags.get("repo").map(String::as_str),
            Some("eink-bridge")
        );
    }

    #[test]
    fn an_empty_session_omits_the_optional_fields_rather_than_sending_nulls() {
        // `origin` and `annotations` are skipped when absent, so a client can tell
        // "no origin recorded" from "origin with every field null".
        let dir = tempfile::tempdir().unwrap();
        let mut manager = SessionManager::new(PathBuf::from(dir.path()));
        let session = manager.create("body".into(), None, None, HashMap::new(), false, None);
        let body = serde_json::to_value(SessionResultResponse::from_session(&session)).unwrap();
        assert!(body.get("origin").is_none());
        assert!(body.get("annotations").is_none());
        assert_eq!(body["has_annotation"], false);
        assert_eq!(body["schema_version"], SCHEMA_VERSION);
    }

    #[test]
    fn legacy_payloads_without_schema_version_are_accepted() {
        // An older CLI or app posts without the version field; the server must keep
        // working rather than reject the review.
        let request: CreateSessionRequest =
            serde_json::from_str(r#"{"content": "hello"}"#).unwrap();
        assert_eq!(request.schema_version, None);
        assert_eq!(request.content, "hello");
        assert!(request.tags.is_empty());

        let review: SubmitReviewRequest =
            serde_json::from_str(r#"{"typed_notes": "LGTM"}"#).unwrap();
        assert_eq!(review.typed_notes, "LGTM");
        assert!(review.annotations.is_empty());
        assert_eq!(review.verdict, None);
    }
}
