use serde::Serialize;
use serde_json::Value;

#[derive(Serialize)]
pub(crate) struct DiagramPayload<'a> {
    pub kind: &'a str,
    pub source: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(serde::Deserialize, Serialize)]
pub(crate) struct MindMapDoc {
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default = "default_true")]
    pub index: bool,
    #[serde(default)]
    pub nodes: Vec<MindMapNode>,
}

#[derive(serde::Deserialize, Serialize)]
pub(crate) struct MindMapNode {
    #[serde(default)]
    pub id: Option<String>,
    pub label: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub href: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub line: Option<u32>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub children: Vec<MindMapNode>,
}

#[derive(serde::Deserialize, Serialize)]
pub(crate) struct GraphDoc {
    #[serde(default)]
    pub layout: Option<GraphLayout>,
    #[serde(default)]
    pub nodes: Vec<GraphNode>,
    #[serde(default)]
    pub edges: Vec<GraphEdge>,
}

#[derive(serde::Deserialize, Serialize)]
pub(crate) struct GraphLayout {
    #[serde(default)]
    pub algorithm: Option<String>,
    #[serde(default)]
    pub direction: Option<String>,
    #[serde(default)]
    pub node_spacing: Option<u32>,
    #[serde(default)]
    pub layer_spacing: Option<u32>,
}

#[derive(serde::Deserialize, Serialize)]
pub(crate) struct GraphNode {
    pub id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub line: Option<u32>,
}

#[derive(serde::Deserialize, Serialize)]
pub(crate) struct GraphEdge {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
}

fn default_true() -> bool {
    true
}
