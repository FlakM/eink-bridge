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
pub struct SubmitReviewRequest {
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
            has_annotation: !session.annotation_images.is_empty(),
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
                    "description": "Supports either plain text markdown body with optional query params or JSON request body.",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": {"$ref": "#/components/schemas/CreateSessionRequest"}
                            },
                            "text/plain": {
                                "schema": {"type": "string"}
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
                        {"type": "object", "properties": {"callback_url": {"type": ["string", "null"]}}}
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
                        "has_annotation": {"type": "boolean"},
                        "review_duration_seconds": {"type": ["integer", "null"]},
                        "tags": {"type": "object", "additionalProperties": {"type": "string"}},
                        "stroke_data": {
                            "description": "Raw pen strokes: {canvas_width, canvas_height, strokes:[[[x,y],...],...]}"
                        },
                        "annotations": {
                            "type": "array",
                            "items": {"$ref": "#/components/schemas/AnnotationGroup"}
                        }
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
