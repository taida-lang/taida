//! Output formatting for graphs: text, JSON, Mermaid, Graphviz DOT.

use super::model::*;

/// Output format.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputFormat {
    Text,
    Json,
    Mermaid,
    Dot,
}

impl OutputFormat {
    pub fn parse(s: &str) -> Option<OutputFormat> {
        match s {
            "text" => Some(OutputFormat::Text),
            "json" => Some(OutputFormat::Json),
            "mermaid" => Some(OutputFormat::Mermaid),
            "dot" => Some(OutputFormat::Dot),
            _ => None,
        }
    }
}

impl std::str::FromStr for OutputFormat {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or(())
    }
}

/// Format a graph to a string.
pub fn format_graph(graph: &Graph, format: OutputFormat) -> String {
    match format {
        OutputFormat::Text => format_text(graph),
        OutputFormat::Json => format_json(graph),
        OutputFormat::Mermaid => format_mermaid(graph),
        OutputFormat::Dot => format_dot(graph),
    }
}

/// Human-readable text format.
fn format_text(graph: &Graph) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Graph: {} ({})\n",
        graph.view,
        graph.source_files.join(", ")
    ));
    out.push_str(&format!(
        "Nodes: {}, Edges: {}\n",
        graph.nodes.len(),
        graph.edges.len()
    ));
    out.push('\n');

    out.push_str("Nodes:\n");
    for node in &graph.nodes {
        out.push_str(&format!(
            "  [{}] {} ({}:{})\n",
            node.kind, node.label, node.location.file, node.location.line
        ));
    }

    out.push('\n');
    out.push_str("Edges:\n");
    for edge in &graph.edges {
        let source_label = graph
            .nodes
            .iter()
            .find(|n| n.id == edge.source)
            .map(|n| n.label.as_str())
            .unwrap_or(&edge.source);
        let target_label = graph
            .nodes
            .iter()
            .find(|n| n.id == edge.target)
            .map(|n| n.label.as_str())
            .unwrap_or(&edge.target);
        out.push_str(&format!(
            "  {} --({})-->> {}\n",
            source_label, edge.kind, target_label
        ));
    }

    out
}

/// JSON format.
fn format_json(graph: &Graph) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!("  \"view\": \"{}\",\n", graph.view));

    // Nodes
    out.push_str("  \"nodes\": [\n");
    for (i, node) in graph.nodes.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!("      \"id\": \"{}\",\n", escape_json(&node.id)));
        out.push_str(&format!("      \"kind\": \"{}\",\n", node.kind));
        out.push_str(&format!(
            "      \"label\": \"{}\",\n",
            escape_json(&node.label)
        ));
        out.push_str("      \"location\": {\n");
        out.push_str(&format!(
            "        \"file\": \"{}\",\n",
            escape_json(&node.location.file)
        ));
        out.push_str(&format!("        \"line\": {},\n", node.location.line));
        out.push_str(&format!("        \"column\": {}\n", node.location.column));
        out.push_str("      },\n");
        out.push_str("      \"metadata\": {");
        let meta_entries: Vec<String> = node
            .metadata
            .iter()
            .map(|(k, v)| format!("\"{}\": \"{}\"", escape_json(k), escape_json(v)))
            .collect();
        out.push_str(&meta_entries.join(", "));
        out.push_str("}\n");
        out.push_str("    }");
        if i < graph.nodes.len() - 1 {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ],\n");

    // Edges
    out.push_str("  \"edges\": [\n");
    for (i, edge) in graph.edges.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!(
            "      \"source\": \"{}\",\n",
            escape_json(&edge.source)
        ));
        out.push_str(&format!(
            "      \"target\": \"{}\",\n",
            escape_json(&edge.target)
        ));
        out.push_str(&format!("      \"kind\": \"{}\",\n", edge.kind));
        out.push_str(&format!(
            "      \"label\": \"{}\",\n",
            escape_json(&edge.label)
        ));
        out.push_str("      \"metadata\": {");
        let meta_entries: Vec<String> = edge
            .metadata
            .iter()
            .map(|(k, v)| format!("\"{}\": \"{}\"", escape_json(k), escape_json(v)))
            .collect();
        out.push_str(&meta_entries.join(", "));
        out.push_str("}\n");
        out.push_str("    }");
        if i < graph.edges.len() - 1 {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ],\n");

    // Metadata
    out.push_str("  \"metadata\": {\n");
    let files_json: Vec<String> = graph
        .source_files
        .iter()
        .map(|f| format!("\"{}\"", escape_json(f)))
        .collect();
    out.push_str(&format!(
        "    \"source_files\": [{}],\n",
        files_json.join(", ")
    ));
    out.push_str("    \"taida_version\": \"0.1.0\"\n");
    out.push_str("  }\n");

    out.push_str("}\n");
    out
}

/// Mermaid diagram format.
fn format_mermaid(graph: &Graph) -> String {
    let mut out = String::new();
    let direction = match graph.view {
        GraphView::TypeHierarchy => "TB",
        _ => "LR",
    };
    out.push_str(&format!("graph {}\n", direction));

    // Nodes
    for node in &graph.nodes {
        let sanitized_id = sanitize_mermaid_id(&node.id);
        let shape = match node.kind {
            NodeKind::Function | NodeKind::AnonymousFn | NodeKind::Entrypoint => {
                format!("{}([\"{}\"]);", sanitized_id, escape_mermaid(&node.label))
            }
            NodeKind::Condition => {
                format!("{}{{\"{}\"}};", sanitized_id, escape_mermaid(&node.label))
            }
            NodeKind::ErrorCeiling | NodeKind::GorillaCeiling => {
                format!("{}>\"{}\"]; ", sanitized_id, escape_mermaid(&node.label))
            }
            _ => format!("{}[\"{}\"]; ", sanitized_id, escape_mermaid(&node.label)),
        };
        out.push_str(&format!("    {}\n", shape.trim()));
    }

    out.push('\n');

    // Edges
    for edge in &graph.edges {
        let source_id = sanitize_mermaid_id(&edge.source);
        let target_id = sanitize_mermaid_id(&edge.target);
        out.push_str(&format!(
            "    {} -->|\"{}\"| {}\n",
            source_id, edge.kind, target_id
        ));
    }

    out
}

/// Graphviz DOT format.
fn format_dot(graph: &Graph) -> String {
    let mut out = String::new();
    let graph_name = match graph.view {
        GraphView::Dataflow => "dataflow",
        GraphView::Module => "module_dependency",
        GraphView::TypeHierarchy => "type_hierarchy",
        GraphView::Error => "error_boundary",
        GraphView::Call => "call_graph",
    };
    out.push_str(&format!("digraph {} {{\n", graph_name));
    out.push_str("    rankdir=LR;\n");
    out.push_str("    node [shape=box];\n\n");

    // Nodes with attributes
    for node in &graph.nodes {
        let sanitized_id = sanitize_dot_id(&node.id);
        let shape = match node.kind {
            NodeKind::Function | NodeKind::AnonymousFn | NodeKind::Entrypoint => "ellipse",
            NodeKind::Condition => "diamond",
            NodeKind::ErrorCeiling | NodeKind::GorillaCeiling => "hexagon",
            _ => "box",
        };
        out.push_str(&format!(
            "    \"{}\" [label=\"{}\", shape={}];\n",
            sanitized_id,
            escape_dot(&node.label),
            shape
        ));
    }

    out.push('\n');

    // Edges
    for edge in &graph.edges {
        let source_id = sanitize_dot_id(&edge.source);
        let target_id = sanitize_dot_id(&edge.target);
        let style = match edge.kind {
            EdgeKind::Imports | EdgeKind::Exports => ", style=dashed",
            EdgeKind::ThrowsTo | EdgeKind::Propagates => ", color=red",
            _ => "",
        };
        out.push_str(&format!(
            "    \"{}\" -> \"{}\" [label=\"{}\"{}];\n",
            source_id,
            escape_dot(&target_id),
            edge.kind,
            style
        ));
    }

    out.push_str("}\n");
    out
}

/// Escape special characters for JSON strings.
fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

/// Escape and sanitize for Mermaid.
fn escape_mermaid(s: &str) -> String {
    s.replace('"', "'")
}

fn sanitize_mermaid_id(s: &str) -> String {
    s.replace([':', '/', '.', ' '], "_")
}

/// Escape for DOT.
fn escape_dot(s: &str) -> String {
    s.replace('"', "\\\"")
}

fn sanitize_dot_id(s: &str) -> String {
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::extract::GraphExtractor;

    fn parse_and_format(source: &str, view: GraphView, format: OutputFormat) -> String {
        let (program, errors) = crate::parser::parse(source);
        assert!(errors.is_empty(), "Parse errors: {:?}", errors);
        let mut extractor = GraphExtractor::new("test.td");
        let graph = extractor.extract(&program, view);
        format_graph(&graph, format)
    }

    #[test]
    fn test_text_format() {
        let output = parse_and_format("x <= 42", GraphView::Dataflow, OutputFormat::Text);
        assert!(output.contains("Graph: dataflow"));
        assert!(output.contains("x"));
        assert!(output.contains("42"));
    }

    #[test]
    fn test_json_format() {
        let output = parse_and_format("x <= 42", GraphView::Dataflow, OutputFormat::Json);
        assert!(output.contains("\"view\": \"dataflow\""));
        assert!(output.contains("\"nodes\""));
        assert!(output.contains("\"edges\""));
    }

    #[test]
    fn test_mermaid_format() {
        let output = parse_and_format("x <= 42", GraphView::Dataflow, OutputFormat::Mermaid);
        assert!(output.contains("graph LR"));
    }

    #[test]
    fn test_dot_format() {
        let output = parse_and_format("x <= 42", GraphView::Dataflow, OutputFormat::Dot);
        assert!(output.contains("digraph dataflow"));
        assert!(output.contains("rankdir=LR"));
    }
}
