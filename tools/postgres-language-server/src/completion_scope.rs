//! Resolve columns from the parser's query scopes and CTE projections.
//! No database access and no SQL execution. Unknown expressions stay unknown.
use super::CatalogSnapshot;
use std::{collections::BTreeMap, ops::Range, sync::Arc};
use tree_sitter::Node;

const MARKER: &str = "__rustable_completion_cursor__";
const DEPTH_LIMIT: usize = 16;
const COLUMN_LIMIT: usize = 2048;

type Relations = BTreeMap<String, Arc<Vec<String>>>;
const BINDING_LIMIT: usize = 64;

fn children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}
fn name(node: Node<'_>, text: &str) -> String {
    let raw = &text[node.byte_range()];
    if raw.starts_with('"') && raw.ends_with('"') && raw.len() >= 2 {
        raw[1..raw.len() - 1].replace("\"\"", "\"")
    } else {
        raw.to_ascii_lowercase()
    }
}
fn is_identifier(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "any_identifier" | "schema_identifier" | "table_identifier" | "column_identifier"
    )
}
fn parts(node: Node<'_>, text: &str) -> Vec<String> {
    children(node)
        .into_iter()
        .filter(|node| is_identifier(*node))
        .map(|node| name(node, text))
        .collect()
}
fn alias(node: Node<'_>, text: &str) -> Option<String> {
    children(node)
        .into_iter()
        .find(|n| n.kind() == "alias")
        .and_then(|n| children(n).into_iter().find(|n| is_identifier(*n)))
        .map(|n| name(n, text))
}
// Alias column lists rename leading output positions; omitted positions retain
// their original names. Do not invent extra columns for an invalid alias list.
fn relation_columns(node: Node<'_>, text: &str, mut columns: Arc<Vec<String>>) -> Arc<Vec<String>> {
    let Some(list) = children(node).into_iter().find(|n| n.kind() == "list") else {
        return columns;
    };
    let declared = children(list)
        .into_iter()
        .filter(|n| n.kind() == "column")
        .filter_map(|n| children(n).into_iter().find(|n| is_identifier(*n)))
        .take(COLUMN_LIMIT)
        .map(|n| name(n, text));
    for (index, declared) in declared.enumerate().take(columns.len()) {
        Arc::make_mut(&mut columns)[index] = declared;
    }
    columns
}
fn query_scope(mut node: Node<'_>) -> Option<Node<'_>> {
    loop {
        if matches!(node.kind(), "statement" | "subquery") {
            return Some(node);
        }
        node = node.parent()?;
    }
}
// Expression subqueries see outer aliases; explicit LATERAL joins see only
// preceding sources. Ordinary derived tables and CTE bodies are boundaries.
fn correlated_parent<'a>(scope: Node<'a>, lateral_sources: &[usize]) -> Option<(Node<'a>, usize)> {
    let mut node = scope.parent()?;
    let mut before = usize::MAX;
    loop {
        if matches!(node.kind(), "lateral_join" | "lateral_cross_join") {
            before = node.start_byte();
        }
        if node.kind() == "relation" && lateral_sources.contains(&node.start_byte()) {
            before = node.start_byte();
        } else if matches!(node.kind(), "relation" | "with_query") {
            return None;
        }
        if matches!(node.kind(), "statement" | "subquery") {
            return Some((node, before));
        }
        node = node.parent()?;
    }
}
fn relations(
    scope: Node<'_>,
    text: &str,
    snapshot: &CatalogSnapshot,
    ctes: &Relations,
    depth: usize,
) -> Vec<(String, Arc<Vec<String>>)> {
    relations_before(
        scope,
        text,
        snapshot,
        ctes,
        depth,
        usize::MAX,
        scope.start_byte(),
    )
}
fn relations_before(
    scope: Node<'_>,
    text: &str,
    snapshot: &CatalogSnapshot,
    ctes: &Relations,
    depth: usize,
    before: usize,
    at: usize,
) -> Vec<(String, Arc<Vec<String>>)> {
    if depth > DEPTH_LIMIT {
        return Vec::new();
    }
    let mut result = Vec::new();
    // UNION/INTERSECT/EXCEPT arms share one grammar statement node. Only
    // the arm containing the cursor contributes FROM bindings. Projection
    // inference passes the scope start to inspect the first (output-name) arm.
    let nodes = children(scope);
    let mut arm_start = scope.start_byte();
    let mut arm_end = scope.end_byte();
    for node in &nodes {
        if matches!(
            node.kind(),
            "keyword_union" | "keyword_intersect" | "keyword_except"
        ) {
            if node.start_byte() < at {
                arm_start = node.end_byte();
            } else {
                arm_end = node.start_byte();
                break;
            }
        }
    }
    let Some(from) = nodes
        .into_iter()
        .find(|n| n.kind() == "from" && n.start_byte() >= arm_start && n.start_byte() < arm_end)
    else {
        return result;
    };
    let mut pending = vec![from];
    while let Some(node) = pending.pop() {
        if node.start_byte() >= before {
            continue;
        }
        if result.len() >= BINDING_LIMIT {
            break;
        }
        if matches!(node.kind(), "lateral_join" | "lateral_cross_join") {
            let nodes = children(node);
            let binding = alias(node, text)
                .or_else(|| node.child_by_field_name("alias").map(|n| name(n, text)));
            if let (Some(query), Some(binding)) =
                (nodes.iter().find(|n| n.kind() == "subquery"), binding)
            {
                result.push((
                    binding,
                    relation_columns(
                        node,
                        text,
                        Arc::new(projection(*query, text, snapshot, ctes, depth + 1)),
                    ),
                ));
            }
        } else if node.kind() == "relation" {
            let nodes = children(node);
            if let Some(reference) = nodes.iter().find(|n| n.kind() == "table_reference") {
                let reference = parts(*reference, text);
                let Some(table) = reference.last() else {
                    continue;
                };
                let columns = if reference.len() == 1 {
                    ctes.get(table)
                        .cloned()
                        .unwrap_or_else(|| snapshot.columns(&reference))
                } else {
                    snapshot.columns(&reference)
                };
                result.push((
                    alias(node, text).unwrap_or_else(|| table.clone()),
                    relation_columns(node, text, columns),
                ));
            } else if let Some(subquery) = nodes.iter().find(|n| n.kind() == "subquery")
                && let Some(alias) = alias(node, text)
            {
                result.push((
                    alias,
                    relation_columns(
                        node,
                        text,
                        Arc::new(projection(*subquery, text, snapshot, ctes, depth + 1)),
                    ),
                ));
            }
            if let Some(values) = nodes.iter().find(|n| n.kind() == "values")
                && let Some(binding) = alias(node, text)
            {
                result.push((
                    binding,
                    relation_columns(node, text, Arc::new(value_columns(*values))),
                ));
            }
            // Never mix a nested FROM's aliases into the surrounding query.
        } else {
            pending.extend(children(node).into_iter().rev());
        }
    }
    result
}
fn value_columns(values: Node<'_>) -> Vec<String> {
    children(values)
        .into_iter()
        .find(|n| n.kind() == "list")
        .map(|list| {
            children(list)
                .iter()
                .filter(|node| !node.is_extra())
                .take(COLUMN_LIMIT)
                .enumerate()
                .map(|(i, _)| format!("column{}", i + 1))
                .collect()
        })
        .unwrap_or_default()
}
fn projection(
    scope: Node<'_>,
    text: &str,
    snapshot: &CatalogSnapshot,
    inherited: &Relations,
    depth: usize,
) -> Vec<String> {
    if depth > DEPTH_LIMIT {
        return vec![];
    }
    let ctes = visible_ctes(scope, usize::MAX, text, snapshot, inherited, depth + 1);
    let relations = relations(scope, text, snapshot, &ctes, depth + 1);
    let Some(select) = children(scope)
        .into_iter()
        .find(|n| matches!(n.kind(), "select" | "values"))
    else {
        return vec![];
    };
    if select.kind() == "values" {
        return value_columns(select);
    }
    let Some(expressions) = children(select)
        .into_iter()
        .find(|n| n.kind() == "select_expression")
    else {
        return vec![];
    };
    let mut columns = Vec::new();
    for term in children(expressions)
        .into_iter()
        .filter(|n| n.kind() == "term")
    {
        if let Some(alias) = alias(term, text) {
            columns.push(alias);
        } else if let Some(expression) = children(term).first() {
            match expression.kind() {
                "object_reference" | "column_reference" => {
                    if let Some(column) = parts(*expression, text).last() {
                        columns.push(column.clone());
                    }
                }
                "all_fields" => {
                    let names = parts(*expression, text);
                    if let Some(qualifier) = names.last() {
                        if let Some(found) = relations
                            .iter()
                            .find(|(name, _)| name == qualifier)
                            .map(|(_, columns)| columns)
                        {
                            columns.extend(
                                found
                                    .iter()
                                    .take(COLUMN_LIMIT.saturating_sub(columns.len()))
                                    .cloned(),
                            );
                        }
                    } else {
                        columns.extend(
                            relations
                                .iter()
                                .flat_map(|(_, v)| v.iter())
                                .take(COLUMN_LIMIT.saturating_sub(columns.len()))
                                .cloned(),
                        );
                    }
                }
                _ => columns.push(String::new()), // Preserve ordinal for later CTE aliases.
            }
        }
        if columns.len() > COLUMN_LIMIT {
            columns.truncate(COLUMN_LIMIT);
            break;
        }
    }
    columns
}
fn visible_ctes(
    scope: Node<'_>,
    before: usize,
    text: &str,
    snapshot: &CatalogSnapshot,
    inherited: &Relations,
    depth: usize,
) -> Relations {
    let mut result = inherited.clone();
    if depth > DEPTH_LIMIT {
        return result;
    }
    let Some(cte) = children(scope).into_iter().find(|n| n.kind() == "cte") else {
        return result;
    };
    let recursive = children(cte)
        .iter()
        .any(|n| n.kind() == "keyword_recursive");
    for query in children(cte)
        .into_iter()
        .filter(|n| n.kind() == "with_query" && n.start_byte() < before)
        .take(BINDING_LIMIT)
    {
        let nodes = children(query);
        let Some(identifier) = nodes.iter().find(|n| is_identifier(**n)) else {
            continue;
        };
        let body = nodes.iter().find(|n| n.kind() == "statement");
        let self_visible = recursive
            && body.is_some_and(|body| {
                children(*body)
                    .iter()
                    .any(|n| n.kind() == "keyword_union" && n.start_byte() < before)
            });
        if query.end_byte() > before && !self_visible {
            continue;
        }
        let mut walk = query.walk();
        let declared: Vec<_> = query
            .children_by_field_name("argument", &mut walk)
            .take(COLUMN_LIMIT)
            .map(|n| name(n, text))
            .collect();
        let mut columns = body
            .map(|body| projection(*body, text, snapshot, &result, depth + 1))
            .unwrap_or_default();
        for (index, declared) in declared.into_iter().enumerate() {
            if index < columns.len() {
                columns[index] = declared;
            } else {
                columns.push(declared);
            }
        }
        result.insert(name(*identifier, text), Arc::new(columns));
    }
    result
}

/// Some means a column expression was resolved, including a known scope
/// with no matching columns. None lets the upstream provider handle the case.
pub(super) fn complete(
    snapshot: &CatalogSnapshot,
    text: &str,
    cursor: usize,
) -> Option<Vec<(String, Range<usize>, String)>> {
    if !text.is_char_boundary(cursor) {
        return None;
    }
    // A temporary identifier makes `alias.` parseable. The original document
    // and edit ranges never contain this marker, including inside quoted names.
    let mut parsed = text.to_owned();
    parsed.insert_str(cursor, MARKER);
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&pgls_treesitter_grammar::LANGUAGE.into())
        .ok()?;
    // The upstream tree grammar does not accept doubled quotes or Unicode in
    // every identifier position. Use its PostgreSQL lexer to mask only those
    // identifier bytes for parsing; names and edits still read original text.
    let lexed = pgls_lexer::lex(&parsed);
    let mut compatible = parsed.as_bytes().to_vec();
    // Normalize LATERAL subqueries to ordinary relations: the upstream grammar
    // lacks comma-form LATERAL and column alias lists on explicit LATERAL joins.
    // Mask only its keyword, keeping all byte/edit offsets,
    // and retain the following subquery start as an explicit scope boundary.
    let significant: Vec<_> = lexed
        .tokens()
        .enumerate()
        .filter(|(_, kind)| !kind.is_trivia())
        .map(|(index, _)| index)
        .collect();
    let mut lateral_sources = Vec::new();
    for tokens in significant.windows(3) {
        let previous = lexed.text(tokens[0]);
        if (previous == ","
            || previous.eq_ignore_ascii_case("FROM")
            || previous.eq_ignore_ascii_case("JOIN"))
            && lexed.text(tokens[1]).eq_ignore_ascii_case("LATERAL")
            && lexed.text(tokens[2]) == "("
        {
            let range = lexed.range(tokens[1]);
            compatible[usize::from(range.start())..usize::from(range.end())].fill(b' ');
            lateral_sources.push(usize::from(lexed.range(tokens[2]).start()));
        }
    }
    for (index, kind) in lexed.tokens().enumerate() {
        if !matches!(
            kind,
            pgls_lexer::SyntaxKind::IDENT | pgls_lexer::SyntaxKind::ERROR
        ) {
            continue;
        }
        let token = lexed.text(index);
        let unicode_identifier =
            kind == pgls_lexer::SyntaxKind::ERROR && token.chars().all(|ch| !ch.is_ascii());
        if unicode_identifier
            || (kind == pgls_lexer::SyntaxKind::IDENT
                && (!token.is_ascii() || token.starts_with('"')))
        {
            let range = lexed.range(index);
            // Bare quoted identifiers are incorrectly parsed as literals by
            // the grammar. Mask their entire token, retaining byte boundaries.
            let start = usize::from(range.start());
            let end = usize::from(range.end());
            compatible[start..end].fill(b'x');
        }
    }
    let tree = parser.parse(&compatible, None)?;

    let mut reference = tree
        .root_node()
        .descendant_for_byte_range(cursor, cursor + MARKER.len())?;
    while !matches!(reference.kind(), "object_reference" | "column_reference") {
        if matches!(
            reference.kind(),
            "literal" | "comment" | "statement" | "subquery"
        ) {
            return None;
        }
        reference = reference.parent()?;
    }
    let identifiers: Vec<_> = children(reference)
        .into_iter()
        .filter(|n| is_identifier(*n))
        .collect();
    if identifiers.is_empty() || identifiers.len() > 2 {
        return None;
    }
    let qualified = identifiers.len() == 2;
    let target = *identifiers.last()?;
    if target.start_byte() > cursor || target.end_byte() < cursor + MARKER.len() {
        return None;
    }
    if reference.parent().is_some_and(|node| node.kind() == "type") {
        return None;
    }
    let scope = query_scope(reference)?;
    let mut parents = vec![scope];
    let mut parent = scope.parent();
    while let Some(node) = parent {
        if matches!(node.kind(), "statement" | "subquery") {
            parents.push(node);
        }
        if parents.len() > DEPTH_LIMIT {
            return None;
        }
        parent = node.parent();
    }
    let mut ctes = Relations::new();
    let mut scope_ctes = Vec::new();
    for parent in parents.iter().rev() {
        ctes = visible_ctes(*parent, cursor, &parsed, snapshot, &ctes, 0);
        scope_ctes.push((*parent, ctes.clone()));
    }
    let mut bindings = relations_before(scope, &parsed, snapshot, &ctes, 0, usize::MAX, cursor);
    if qualified {
        let qualifier = name(identifiers[0], &parsed);
        let mut outer = scope;
        while !bindings.iter().any(|(name, _)| name == &qualifier) {
            let Some((parent, before)) = correlated_parent(outer, &lateral_sources) else {
                break;
            };
            let (_, ctes) = scope_ctes.iter().find(|(node, _)| *node == parent)?;
            bindings = relations_before(
                parent,
                &parsed,
                snapshot,
                ctes,
                0,
                before,
                outer.start_byte(),
            );
            outer = parent;
        }
    }
    let mut outer_ambiguous = std::collections::BTreeSet::new();
    if !qualified {
        // PostgreSQL resolves each bare column at the nearest query level.
        // Collect outer names only when no closer source defines that name.
        let mut seen_columns = std::collections::BTreeSet::new();
        let mut seen_aliases = std::collections::BTreeSet::new();
        let mut level = bindings.clone();
        let mut outer = scope;
        loop {
            for (alias, columns) in &level {
                seen_aliases.insert(alias.clone());
                seen_columns.extend(columns.iter().cloned());
            }
            let Some((parent, before)) = correlated_parent(outer, &lateral_sources) else {
                break;
            };
            let (_, ctes) = scope_ctes.iter().find(|(node, _)| *node == parent)?;
            level = relations_before(
                parent,
                &parsed,
                snapshot,
                ctes,
                0,
                before,
                outer.start_byte(),
            );
            let mut counts = BTreeMap::new();
            for (_, columns) in &level {
                for column in columns.iter().collect::<std::collections::BTreeSet<_>>() {
                    *counts.entry(column.clone()).or_insert(0usize) += 1;
                }
            }
            outer_ambiguous.extend(
                counts
                    .iter()
                    .filter(|(column, count)| **count > 1 && !seen_columns.contains(*column))
                    .map(|(column, _)| column.clone()),
            );
            for (alias, columns) in &level {
                let visible = columns
                    .iter()
                    .filter(|column| {
                        !seen_columns.contains(*column)
                        // An ambiguous outer name needs a visible qualifier.
                        && !(counts.get(*column).is_some_and(|count| *count > 1)
                            && seen_aliases.contains(alias))
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                if !visible.is_empty() {
                    bindings.push((alias.clone(), Arc::new(visible)));
                }
            }
            outer = parent;
        }
    }
    let sources: Vec<_> = if qualified {
        let qualifier = name(identifiers[0], &parsed);
        vec![bindings.iter().find(|(name, _)| name == &qualifier)?]
    } else {
        bindings.iter().collect()
    };
    // Count source bindings, not repeated projection names. Ambiguous names
    // must insert a qualifier instead of producing invalid bare references.
    let mut occurrences = BTreeMap::new();
    if !qualified {
        for (_, columns) in &sources {
            let unique: std::collections::BTreeSet<_> = columns.iter().collect();
            for column in unique {
                *occurrences.entry(column).or_insert(0usize) += 1;
            }
        }
    }
    let range = target.start_byte()..target.end_byte() - MARKER.len();
    let raw_prefix = text.get(range.start..cursor)?;
    let quoted = raw_prefix.starts_with('"');
    let prefix = if quoted {
        raw_prefix[1..].replace("\"\"", "\"")
    } else {
        raw_prefix.to_ascii_lowercase()
    };
    let mut found = Vec::new();
    for (source, columns) in sources {
        for column in columns.iter() {
            let matches = if quoted {
                column.starts_with(&prefix)
            } else {
                column.to_ascii_lowercase().starts_with(&prefix)
            };
            let ambiguous = occurrences.get(column).is_some_and(|count| *count > 1)
                || outer_ambiguous.contains(column);
            let label = if ambiguous {
                format!("{source}.{column}")
            } else {
                column.clone()
            };
            if !column.is_empty() && matches && !found.iter().any(|(name, _, _)| name == &label) {
                let column_sql = format!("\"{}\"", column.replace('"', "\"\""));
                let replacement = if ambiguous {
                    format!("\"{}\".{column_sql}", source.replace('"', "\"\""))
                } else {
                    column_sql
                };
                found.push((label, range.clone(), replacement));
                if found.len() == 50 {
                    return Some(found);
                }
            }
        }
    }
    Some(found) // Known scope, even when no column matches the prefix.
}
