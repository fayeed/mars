use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainSymbol {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub qualified_name: String,
    pub relative_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub exported: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EdgeEvidence {
    pub relative_path: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub parser: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainEdge {
    pub id: String,
    pub edge_kind: String,
    pub source: Option<BrainSymbol>,
    pub target: Option<BrainSymbol>,
    pub target_key: String,
    pub confidence: f64,
    pub evidence: EdgeEvidence,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainNeighborhood {
    pub root: BrainSymbol,
    pub symbols: Vec<BrainSymbol>,
    pub edges: Vec<BrainEdge>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dependency {
    pub relative_path: String,
    pub specifier: String,
    pub confidence: f64,
    pub evidence: EdgeEvidence,
}
