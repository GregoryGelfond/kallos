//! The concrete-syntax-tree layer: parsing and parse-status.

use tree_sitter::{Language, Node, Parser, Tree};

/// Parse ASP/clingo source into a tree-sitter concrete syntax tree.
///
/// Total: tree-sitter is error-tolerant and always returns a [`Tree`]. Invalid
/// input is represented *within* the tree as `ERROR`/`MISSING` nodes rather than
/// by a failure, so parse-status is a query on the returned tree (`has_error`)
/// rather than something signalled here.
///
/// # Panics
///
/// Panics only on a violated internal invariant, never on caller input:
///
/// - if the vendored `tree-sitter-clingo` grammar fails to load into the
///   [`Parser`] — the grammar is compiled into this binary, so a failure here is
///   a broken build, not bad input;
/// - if tree-sitter returns no tree for `&str` input — it always returns one,
///   reserving `None` for a configured timeout or cancellation flag, neither of
///   which this call sets.
// `parse` is the formatter's CST primitive. It returns `tree_sitter::Tree`, which
// must not leak into the public API, so it stays `pub(crate)`; the
// public parse-status surface is `has_error` below, which is its first non-test
// consumer (so `parse` is now live in the library build).
#[must_use]
pub(crate) fn parse(src: &str) -> Tree {
    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_clingo::LANGUAGE.into();
    parser
        .set_language(&language)
        .expect("the vendored tree-sitter-clingo grammar must load");
    parser
        .parse(src, None)
        .expect("tree-sitter always returns a tree for &str input")
}

/// Does this source's parse contain `ERROR` (or `MISSING`) nodes? Mirrors
/// tree-sitter's [`tree_sitter::Node::has_error`]. The CLI's conservative-skip
/// and the LSP's decline-to-format-on-save key on this — NOT on
/// inspecting `format`'s output. Scoped to a `bool`: error *locations* are a
/// diagnostics concern, not the formatter's.
#[must_use]
pub fn has_error(src: &str) -> bool {
    parse(src).root_node().has_error()
}

/// Cached `u16` kind ids for the constructs the emitter and `≈` dispatch on.
///
/// Resolved once from the grammar (an integer jump-table beats
/// repeated `&str` `kind()` comparisons). `node-types.json` is the build-time
/// exhaustiveness reference; the runtime structure the emitter walks is the
/// cursor child sequence (see [`walk_children`]) keyed by these ids.
pub struct KindIds {
    // The translation unit (root) — its statement-sequence layout.
    pub source_file: u16,
    // Statements.
    pub rule: u16,
    pub integrity_constraint: u16,
    pub weak_constraint: u16,
    // NOTE: there is no single `optimize` kind in the grammar. The
    // optimization statements are two distinct named kinds, `minimize` and
    // `maximize` (each with an `optimize_elements` element list of `;`-separated
    // `optimize_element` weight tuples); there is no unifying `optimize` node or
    // supertype. Laid out as bracketed `{ … }.` statements. Verified
    // against node-types.json (2026-06-15).
    pub minimize: u16,
    pub maximize: u16,
    pub show: u16,
    pub show_term: u16,
    pub show_signature: u16,
    pub defined: u16,
    pub edge: u16,
    pub heuristic: u16,
    pub project_signature: u16,
    pub project_atom: u16,
    pub const_: u16,
    pub script: u16,
    pub include: u16,
    pub program: u16,
    pub external: u16,
    pub theory: u16,
    // Heads / bodies / terms (the ones with layout rules).
    pub body: u16,
    pub body_literal: u16,
    pub disjunction: u16,
    pub set_aggregate: u16,
    pub body_aggregate: u16,
    pub head_aggregate: u16,
    pub theory_atom: u16,
    // Aggregate bounds: `lower` (`term relation?`) and `upper` (`relation? term`).
    pub lower: u16,
    pub upper: u16,
    // The aggregate element lists (`;`-separated): each routes through the
    // separated-sequence rule, descended one bracket level inside its braces.
    pub set_aggregate_elements: u16,
    pub head_aggregate_elements: u16,
    pub body_aggregate_elements: u16,
    pub theory_elements: u16,
    pub optimize_elements: u16,
    // The SINGULAR aggregate elements, all the conditional-literal `… : condition` family
    // (lowered through `lower_conditional`, so the condition hangs and breaks under
    // recursive-minimal): `set_aggregate_element` (`literal : condition`),
    // `body_aggregate_element` (`terms : condition`), `head_aggregate_element`
    // (`terms : literal [: condition]` — a leading `:` connective, a later `:` a spaced
    // separator), and `optimize_element` (`weight , terms : condition` — the spaced comma
    // prefix rides the neck).
    pub set_aggregate_element: u16,
    pub body_aggregate_element: u16,
    pub head_aggregate_element: u16,
    pub optimize_element: u16,
    pub conditional_literal: u16,
    // The conditional literal's `condition`: a `,`-separated `literal` list that
    // hangs one level under the conditional `:`, routed through the same
    // separated-sequence rule as bodies / disjunctions / aggregate element lists.
    pub condition: u16,
    pub comparison: u16,
    pub relation: u16, // the comparison operator (`<`, `<=`, `=`, …), spaced
    pub function: u16,
    pub external_function: u16,
    pub binary_operation: u16,
    pub unary_operation: u16,
    pub abs: u16,
    pub tuple: u16,
    // Theory interiors (terms, elements, the `op term` upper, and the
    // `#theory` operator/term/atom definitions).
    pub theory_function: u16,
    pub theory_tuple: u16,
    pub theory_list: u16,
    pub theory_set: u16,
    pub theory_unparsed_term: u16,
    pub theory_terms: u16,
    pub theory_operators: u16,
    pub theory_element: u16,
    pub theory_atom_upper: u16,
    pub theory_operator_definition: u16,
    pub theory_operator_definitions: u16,
    pub theory_term_definition: u16,
    pub theory_atom_definition: u16,
    // (the `theory_operator_arity` / `theory_operator_associativity` / `theory_atom_type`
    // tokens are always-verbatim definition leaves — no kind dispatch is needed on them.)
    // Directives — the sub-node kinds with their own spacing.
    pub signature: u16,  // `name / arity` (defined / show / project) — `/` tight
    pub weight: u16,     // `term @ priority` (weak constraint / heuristic) — `@` tight
    pub parameters: u16, // `id, id, …` (#program) — a `,`-list
    pub edge_pair: u16,  // `term, term` (#edge) — `,` spaced
    // The applicative term/atom layer (terms & pools).
    pub literal: u16,         // head / disjunct / condition literal (sign + atom)
    pub symbolic_atom: u16,   // `[-]name(args)` at literal position
    pub terms: u16,           // a `,`-separated argument / tuple item list
    pub lone_comma: u16,      // a bare `,` pool / tuple item (totality edge case)
    pub empty_pool_item: u16, // an empty pool segment (totality edge case)
    // Word-token leaves — the operand boundary tokens `tok_of` classes for the
    // operator seam asserts (`Ident` / `Number`).
    pub identifier: u16,
    pub variable: u16,
    pub number: u16,
    // Verbatim / opaque spans.
    pub block_comment: u16,
    pub line_comment: u16,
    pub code: u16,
    // Anonymous author-significant tokens.
    pub dot: u16,       // "."
    pub neck: u16,      // ":-"
    pub weak_neck: u16, // ":~"
    pub colon: u16,     // ":"
    pub lbrace: u16,    // "{"  — the aggregate / choice bracket opener
    pub rbrace: u16,    // "}"  — its matching closer
    pub pipe: u16,      // "|"  — the disjunction separator (spaced both sides)
    pub lparen: u16,    // "("  — the term / tuple paren opener (Pad::Tight)
    pub rparen: u16,    // ")"  — its matching closer
    pub semicolon: u16, // ";"  — the argument / tuple pool separator
    pub comma: u16,     // ","  — the field separator in `#theory` definitions
    pub lbracket: u16,  // "["  — the theory-list bracket opener (Pad::Tight)
    pub rbracket: u16,  // "]"  — its matching closer
    pub equals: u16,    // "="  — the #const binding (spaced both sides)
}

impl KindIds {
    /// Resolve every kind id once from the vendored grammar. An unknown kind name
    /// resolves to `0` (tree-sitter's not-found sentinel); the kind-ids test
    /// asserts the load-bearing ids are non-zero, so a typo here is caught.
    #[must_use]
    pub fn new() -> Self {
        let lang: Language = tree_sitter_clingo::LANGUAGE.into();
        let named = |n: &str| lang.id_for_node_kind(n, true);
        let anon = |n: &str| lang.id_for_node_kind(n, false);
        Self {
            source_file: named("source_file"),
            rule: named("rule"),
            integrity_constraint: named("integrity_constraint"),
            weak_constraint: named("weak_constraint"),
            minimize: named("minimize"),
            maximize: named("maximize"),
            show: named("show"),
            show_term: named("show_term"),
            show_signature: named("show_signature"),
            defined: named("defined"),
            edge: named("edge"),
            heuristic: named("heuristic"),
            project_signature: named("project_signature"),
            project_atom: named("project_atom"),
            const_: named("const"),
            script: named("script"),
            include: named("include"),
            program: named("program"),
            external: named("external"),
            theory: named("theory"),
            body: named("body"),
            body_literal: named("body_literal"),
            disjunction: named("disjunction"),
            set_aggregate: named("set_aggregate"),
            body_aggregate: named("body_aggregate"),
            head_aggregate: named("head_aggregate"),
            theory_atom: named("theory_atom"),
            lower: named("lower"),
            upper: named("upper"),
            set_aggregate_elements: named("set_aggregate_elements"),
            head_aggregate_elements: named("head_aggregate_elements"),
            body_aggregate_elements: named("body_aggregate_elements"),
            theory_elements: named("theory_elements"),
            optimize_elements: named("optimize_elements"),
            set_aggregate_element: named("set_aggregate_element"),
            body_aggregate_element: named("body_aggregate_element"),
            head_aggregate_element: named("head_aggregate_element"),
            optimize_element: named("optimize_element"),
            conditional_literal: named("conditional_literal"),
            condition: named("condition"),
            comparison: named("comparison"),
            relation: named("relation"),
            function: named("function"),
            external_function: named("external_function"),
            binary_operation: named("binary_operation"),
            unary_operation: named("unary_operation"),
            abs: named("abs"),
            tuple: named("tuple"),
            theory_function: named("theory_function"),
            theory_tuple: named("theory_tuple"),
            theory_list: named("theory_list"),
            theory_set: named("theory_set"),
            theory_unparsed_term: named("theory_unparsed_term"),
            theory_terms: named("theory_terms"),
            theory_operators: named("theory_operators"),
            theory_element: named("theory_element"),
            theory_atom_upper: named("theory_atom_upper"),
            theory_operator_definition: named("theory_operator_definition"),
            theory_operator_definitions: named("theory_operator_definitions"),
            theory_term_definition: named("theory_term_definition"),
            theory_atom_definition: named("theory_atom_definition"),
            signature: named("signature"),
            weight: named("weight"),
            parameters: named("parameters"),
            edge_pair: named("edge_pair"),
            literal: named("literal"),
            symbolic_atom: named("symbolic_atom"),
            terms: named("terms"),
            lone_comma: named("lone_comma"),
            empty_pool_item: named("empty_pool_item"),
            identifier: named("identifier"),
            variable: named("variable"),
            number: named("number"),
            block_comment: named("block_comment"),
            line_comment: named("line_comment"),
            code: named("code"),
            dot: anon("."),
            neck: anon(":-"),
            weak_neck: anon(":~"),
            colon: anon(":"),
            lbrace: anon("{"),
            rbrace: anon("}"),
            pipe: anon("|"),
            lparen: anon("("),
            rparen: anon(")"),
            semicolon: anon(";"),
            comma: anon(","),
            lbracket: anon("["),
            rbracket: anon("]"),
            equals: anon("="),
        }
    }
}

/// Visit EVERY child of `node` in source order — named AND anonymous — handing
/// each visit its field label (if the grammar assigns one) and the child node.
///
/// This is the load-bearing traversal for `emit` and `≈`: a `child_by_field_name`
/// walk silently drops information the formatter must preserve. Two distinct
/// losses:
///
/// 1. A pool `f(a; b)` surfaces the SAME `arguments` field repeatedly; a
///    field→node map keeps only one segment.
/// 2. Author-significant tokens are anonymous: the `#minimize`/`#minimise`
///    keyword, the `,`/`;`/`|` separators, the brackets. A named-only walk never
///    sees them.
///
/// `node-types.json` documents only the named tree; the real CST the cursor walks
/// has more. The closure is the visitor so the caller never holds a borrow of the
/// internal [`tree_sitter::TreeCursor`].
// First library-build consumer is the `comments` pre-pass (`comments::collect`);
// also consumed by emit and equiv. The dead-code guard has
// self-removed now that a non-test caller exists.
pub fn walk_children<'t>(node: Node<'t>, mut visit: impl FnMut(Option<&str>, Node<'t>)) {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            visit(cursor.field_name(), cursor.node());
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

/// A node is a leaf iff it has no children. This holds uniformly for an anonymous
/// token (`:-`) and a named terminal (`number`), so the emitter treats both the
/// same: a leaf is emitted verbatim via `token()`.
// Consumed by emit's `lower_node`; the dead-code guard has self-removed.
#[must_use]
pub fn is_leaf(node: Node) -> bool {
    node.child_count() == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_trivial_rule() {
        let tree = parse("a :- b.\n");
        let root = tree.root_node();
        assert_eq!(root.kind(), "source_file");
        assert!(!root.has_error(), "a valid rule must parse without ERROR");
    }

    #[test]
    fn marks_invalid_input_with_error() {
        let tree = parse("a :- :- .\n");
        assert!(
            tree.root_node().has_error(),
            "malformed input must carry ERROR"
        );
    }

    #[test]
    fn has_error_mirrors_node_has_error() {
        assert!(!has_error("a :- b.\n"));
        assert!(has_error("a :- :- .\n"));
        assert!(has_error("p(.\n")); // MISSING-bearing
    }

    #[test]
    fn kind_ids_resolve_for_load_bearing_kinds() {
        let k = KindIds::new();
        // named kinds present in the grammar:
        assert_ne!(k.rule, 0);
        assert_ne!(k.integrity_constraint, 0);
        assert_ne!(k.function, 0);
        // the anonymous statement-terminator dot is a token kind:
        assert_ne!(k.dot, 0);
        // distinct kinds must have distinct ids:
        assert_ne!(k.rule, k.function);
    }

    /// Depth-first search for the first node of `kind`, walking ALL children
    /// (named and anonymous) so anonymous tokens like `.` are reachable.
    fn find_first<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
        if node.kind() == kind {
            return Some(node);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) = find_first(child, kind) {
                return Some(found);
            }
        }
        None
    }

    #[test]
    fn walk_visits_anonymous_tokens_and_repeated_pool_fields() {
        // A pool `f(a; b)` surfaces `arguments` MULTIPLE times plus anonymous
        // `(`/`;`/`)`. In this grammar the pool lives on the `symbolic_atom`
        // node, not `function`: `f(a; b)` parses as
        //   symbolic_atom(name: identifier, "(", arguments: terms, ";",
        //                 arguments: terms, ")")
        // while the inner `a`/`b` are 0-ary `function name: (identifier)` nodes.
        // (Verified against the vendored grammar: `symbolic_atom`, not `function`,
        // is where this grammar carries the pool.)
        let tree = parse("f(a; b).\n");
        let root = tree.root_node();
        // descend to the symbolic_atom and collect its child (field, kind) labels:
        let mut labels = Vec::new();
        let atom = find_first(root, "symbolic_atom").expect("a symbolic_atom node");
        walk_children(atom, |field, child| {
            labels.push((field.map(str::to_string), child.kind().to_string()));
        });
        // the '(' , ')' anonymous tokens AND a ';' separator must appear:
        assert!(labels.iter().any(|(_, k)| k == "("));
        assert!(labels.iter().any(|(_, k)| k == ")"));
        assert!(labels.iter().any(|(_, k)| k == ";"));
        // and the 'arguments' field must appear more than once (pool segments):
        let args = labels
            .iter()
            .filter(|(f, _)| f.as_deref() == Some("arguments"))
            .count();
        assert!(
            args >= 2,
            "pool segments surface as repeated `arguments` fields, got {args}"
        );
    }

    #[test]
    fn is_leaf_distinguishes_tokens_from_branches() {
        let tree = parse("f(a).\n");
        let root = tree.root_node();
        let atom = find_first(root, "symbolic_atom").expect("a symbolic_atom node");
        assert!(!is_leaf(atom), "a symbolic_atom is a branch with children");
        let dot = find_first(root, ".").expect("the statement-terminator dot");
        assert!(is_leaf(dot), "an anonymous token has no children");
    }
}
