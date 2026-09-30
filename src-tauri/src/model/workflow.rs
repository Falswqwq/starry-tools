//! 工作流的数据模型 —— 这是落盘到磁盘的形状。

use serde::{Deserialize, Serialize};

use super::params::Params;

pub const CURRENT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

/// 画布上的一个节点实例。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeInstance {
    pub id: String,
    /// 引用某个 [`crate::model::node_kind::NodeKind::id`]。
    pub kind: String,
    #[serde(default)]
    pub position: Position,
    #[serde(default)]
    pub params: Params,
}

/// 一条连线。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Edge {
    pub id: String,
    pub source: String,
    pub source_port: String,
    pub target: String,
    pub target_port: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub nodes: Vec<NodeInstance>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    /// Unix 毫秒时间戳。
    #[serde(default)]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub updated_at: Option<i64>,
}

fn default_version() -> u32 {
    CURRENT_VERSION
}

impl Workflow {
    pub fn new(name: impl Into<String>) -> Self {
        let now = now_millis();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            description: String::new(),
            version: CURRENT_VERSION,
            nodes: Vec::new(),
            edges: Vec::new(),
            created_at: Some(now),
            updated_at: Some(now),
        }
    }

    pub fn node(&self, node_id: &str) -> Option<&NodeInstance> {
        self.nodes.iter().find(|node| node.id == node_id)
    }

    /// 指向 `node_id` 的所有入边。
    pub fn incoming<'a>(&'a self, node_id: &'a str) -> impl Iterator<Item = &'a Edge> + 'a {
        self.edges.iter().filter(move |edge| edge.target == node_id)
    }

    /// 从 `node_id` 出发的所有出边。
    pub fn outgoing<'a>(&'a self, node_id: &'a str) -> impl Iterator<Item = &'a Edge> + 'a {
        self.edges.iter().filter(move |edge| edge.source == node_id)
    }
}

/// 工作流列表里的一行。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub node_count: usize,
    pub edge_count: usize,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
}

impl From<&Workflow> for WorkflowSummary {
    fn from(workflow: &Workflow) -> Self {
        Self {
            id: workflow.id.clone(),
            name: workflow.name.clone(),
            description: workflow.description.clone(),
            node_count: workflow.nodes.len(),
            edge_count: workflow.edges.len(),
            created_at: workflow.created_at,
            updated_at: workflow.updated_at,
        }
    }
}

pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let mut workflow = Workflow::new("测试");
        workflow.nodes.push(NodeInstance {
            id: "n1".into(),
            kind: "input".into(),
            position: Position { x: 10.0, y: 20.0 },
            params: Params::new(),
        });
        workflow.edges.push(Edge {
            id: "e1".into(),
            source: "n1".into(),
            source_port: "out".into(),
            target: "n2".into(),
            target_port: "image".into(),
        });

        let json = serde_json::to_string(&workflow).unwrap();
        assert!(json.contains("\"sourcePort\""));
        assert!(json.contains("\"createdAt\""));

        let back: Workflow = serde_json::from_str(&json).unwrap();
        assert_eq!(back.nodes.len(), 1);
        assert_eq!(back.edges[0].target_port, "image");
        assert_eq!(back.version, CURRENT_VERSION);
    }
}
