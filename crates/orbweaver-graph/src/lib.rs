//! The ecosystem graph, projected from repositories and their resolved
//! dependency edges (directive section 9). Phase I only populates
//! `Repository --depends_on--> Repository`; capability-level edges
//! (`exposes`, `enhances`, `duplicates`, `could_enable`, ...) are added
//! once capability extraction (Phase II) exists to produce them —
//! inventing empty edge kinds now would just be unused scaffolding.

use orbweaver_core::Repository;
use petgraph::graph::{DiGraph, NodeIndex};
use serde::Serialize;
use std::collections::HashMap;

pub struct EcosystemGraph {
    graph: DiGraph<String, DependencyEdge>,
    index_by_repo: HashMap<String, NodeIndex>,
}

#[derive(Debug, Clone)]
pub struct DependencyEdge {
    pub dependency_name: String,
    pub is_path_dependency: bool,
}

impl EcosystemGraph {
    pub fn from_repositories(repositories: &[Repository]) -> Self {
        let mut graph = DiGraph::new();
        let mut index_by_repo = HashMap::new();

        for repo in repositories {
            let idx = graph.add_node(repo.id.clone());
            index_by_repo.insert(repo.id.clone(), idx);
        }

        for repo in repositories {
            let Some(&from) = index_by_repo.get(&repo.id) else {
                continue;
            };
            for dep in &repo.dependencies {
                let Some(target_id) = &dep.resolved_repo else {
                    continue;
                };
                let Some(&to) = index_by_repo.get(target_id) else {
                    continue;
                };
                graph.add_edge(
                    from,
                    to,
                    DependencyEdge {
                        dependency_name: dep.name.clone(),
                        is_path_dependency: dep.is_path_dependency,
                    },
                );
            }
        }

        Self {
            graph,
            index_by_repo,
        }
    }

    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// Repositories that at least one other discovered repository declares
    /// a resolved dependency on — a cheap proxy for "reused infrastructure"
    /// until Phase II's real reuse/multiplicity scoring exists.
    pub fn most_depended_on(&self, top_n: usize) -> Vec<(String, usize)> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for edge in self.graph.edge_indices() {
            if let Some((_, to)) = self.graph.edge_endpoints(edge) {
                *counts.entry(self.graph[to].clone()).or_insert(0) += 1;
            }
        }
        let mut counts: Vec<_> = counts.into_iter().collect();
        counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        counts.truncate(top_n);
        counts
    }

    pub fn to_export(&self) -> GraphExport {
        let nodes = self
            .index_by_repo
            .keys()
            .map(|id| GraphNode { id: id.clone() })
            .collect();

        let edges = self
            .graph
            .edge_indices()
            .filter_map(|e| {
                let (from, to) = self.graph.edge_endpoints(e)?;
                let weight = &self.graph[e];
                Some(GraphEdge {
                    from: self.graph[from].clone(),
                    to: self.graph[to].clone(),
                    relationship: "depends_on".to_string(),
                    dependency_name: weight.dependency_name.clone(),
                    is_path_dependency: weight.is_path_dependency,
                })
            })
            .collect();

        GraphExport { nodes, edges }
    }
}

#[derive(Debug, Serialize)]
pub struct GraphNode {
    pub id: String,
}

#[derive(Debug, Serialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub relationship: String,
    pub dependency_name: String,
    pub is_path_dependency: bool,
}

#[derive(Debug, Serialize)]
pub struct GraphExport {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}
