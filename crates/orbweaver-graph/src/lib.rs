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

#[cfg(test)]
mod tests {
    use super::*;
    use orbweaver_core::DependencyRef;
    use std::path::PathBuf;

    fn repo(id: &str, deps: Vec<DependencyRef>) -> Repository {
        Repository {
            id: id.to_string(),
            name: id.to_string(),
            path: PathBuf::from(id),
            description: None,
            primary_language: None,
            manifests: vec![],
            license: None,
            readme_present: false,
            is_git_repo: false,
            default_branch: None,
            last_commit_at: None,
            commit_count: None,
            contributor_count: None,
            dependencies: deps,
        }
    }

    fn resolved_dep(name: &str, target: &str) -> DependencyRef {
        DependencyRef {
            name: name.to_string(),
            version_req: None,
            manifest: orbweaver_core::ManifestKind::Cargo,
            is_path_dependency: true,
            path_hint: None,
            resolved_repo: Some(target.to_string()),
        }
    }

    #[test]
    fn unresolved_dependencies_do_not_become_edges() {
        let unresolved = DependencyRef {
            name: "serde".to_string(),
            version_req: None,
            manifest: orbweaver_core::ManifestKind::Cargo,
            is_path_dependency: false,
            path_hint: None,
            resolved_repo: None,
        };
        let repos = vec![repo("a", vec![unresolved])];
        let graph = EcosystemGraph::from_repositories(&repos);
        assert_eq!(graph.node_count(), 1);
        assert_eq!(graph.edge_count(), 0);
    }

    #[test]
    fn most_depended_on_ranks_by_incoming_edge_count() {
        let repos = vec![
            repo("a", vec![resolved_dep("core", "core")]),
            repo("b", vec![resolved_dep("core", "core")]),
            repo("c", vec![resolved_dep("core", "core")]),
            repo("core", vec![]),
        ];
        let graph = EcosystemGraph::from_repositories(&repos);
        assert_eq!(graph.node_count(), 4);
        assert_eq!(graph.edge_count(), 3);

        let top = graph.most_depended_on(5);
        assert_eq!(top[0], ("core".to_string(), 3));
    }

    #[test]
    fn export_round_trips_node_and_edge_counts() {
        let repos = vec![
            repo("a", vec![resolved_dep("core", "core")]),
            repo("core", vec![]),
        ];
        let graph = EcosystemGraph::from_repositories(&repos);
        let export = graph.to_export();
        assert_eq!(export.nodes.len(), 2);
        assert_eq!(export.edges.len(), 1);
        assert_eq!(export.edges[0].from, "a");
        assert_eq!(export.edges[0].to, "core");
    }
}
