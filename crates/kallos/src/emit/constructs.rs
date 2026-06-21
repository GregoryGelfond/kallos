//! Per-construct role assignment. Each construct family flips its
//! kinds on in [`handles`], assigns [`super::assemble`] roles in [`build`], and calls
//! the assembler; a kind no family handles falls to the verbatim identity. The
//! families cover rules & constraints, heads (disjunction & bracketed aggregates),
//! body literals & signs & comparisons, terms & pools (applicative atoms/functions,
//! argument lists, pools, tuples, abs, and FLAT term/interval operators), theory
//! interiors, and directives; verbatim spans / escapes fall to the identity.

use super::assemble::{assemble, OpenerKind, Pad, Part, Role};
use super::spacing::{
    comma_spacing, sign_spacing, term_operator_spacing, theory_operator_spacing, Sep,
};
use super::{token, Ctx};
use crate::fusion::{fuses, Class, Tok};
use crate::style::Style;
use doc::{DocBuilder, NodeId};
use rustc_hash::FxHashMap;
use tree_sitter::Node;

/// Does a construct rule exist for `node`'s kind? The spine consults this on the
/// `Pre` visit to decide whether to descend (a handled composite) or emit the node
/// verbatim whole (everything else). The foundation handles nothing — every kind
/// falls to the verbatim default — so the walk is the identity; each
/// family flips its kinds on here, keyed off `node.kind_id()` against `ctx.kinds`.
///
/// INVARIANT: this stays a pure, allocation-free predicate on `node`'s KIND alone.
/// It must not inspect children — descent-worthiness is a function of kind, and it
/// is consulted on every node's `Pre` (the hot descent-decision path). A "handle a
/// rule only if it has a body" refinement belongs in [`build`] (return `None`).
pub(super) fn handles(ctx: &Ctx, node: Node) -> bool {
    let k = ctx.kinds;
    let id = node.kind_id();
    // The translation unit, headed rules, integrity constraints, the rule
    // body. Heads: the disjunctive head and the bracketed aggregates —
    // the choice `set_aggregate`, the optimization `minimize` / `maximize`, each
    // plus its `;`-separated element list (another separated sequence). Everything
    // else falls to the verbatim default.
    id == k.source_file
        || id == k.rule
        || id == k.integrity_constraint
        || id == k.body
        || id == k.body_literal
        || id == k.conditional_literal
        || id == k.condition
        || id == k.disjunction
        || id == k.set_aggregate
        || id == k.head_aggregate
        || id == k.body_aggregate
        || id == k.theory_atom
        || id == k.minimize
        || id == k.maximize
        || id == k.set_aggregate_elements
        || id == k.head_aggregate_elements
        || id == k.body_aggregate_elements
        || id == k.theory_elements
        || id == k.optimize_elements
        // The singular conditional-literal `… : condition` elements; descended so the
        // element's condition can hang and break, not degrade to an unbreakable
        // verbatim leaf. (head/optimize carry a richer prefix — a second `:` guard, a
        // `weight , terms` comma sequence — handled by the same lowering.)
        || id == k.set_aggregate_element
        || id == k.body_aggregate_element
        || id == k.head_aggregate_element
        || id == k.optimize_element
        || id == k.comparison
        // Aggregate bounds (`lower` / `upper`): descended so the bound's relation is
        // lowered spaced, not echoed as a verbatim source span.
        || id == k.lower
        || id == k.upper
        // Terms & pools: the head/disjunct/condition `literal`, the
        // applicative atoms/functions (`symbolic_atom`, `function`,
        // `external_function`), and the `,`-list argument node (`terms`).
        || id == k.literal
        || id == k.symbolic_atom
        || id == k.function
        || id == k.external_function
        || id == k.terms
        || id == k.tuple
        || id == k.binary_operation
        || id == k.unary_operation
        || id == k.abs
        // Theory interiors: the comma-list `theory_terms`, the
        // `theory_terms : condition` element, and the always-spaced operator layer.
        || id == k.theory_terms
        || id == k.theory_element
        || id == k.theory_unparsed_term
        || id == k.theory_operators
        || id == k.theory_function
        || id == k.theory_tuple
        || id == k.theory_list
        || id == k.theory_set
        || id == k.theory_atom_upper
        // #theory definitions: the directive, the term/operator definitions,
        // and the `;`-separated operator-definition list.
        || id == k.theory
        || id == k.theory_term_definition
        || id == k.theory_operator_definition
        || id == k.theory_operator_definitions
        || id == k.theory_atom_definition
        // Directives. The flat keyword + operand(s) [+ `[…]` tail] forms,
        // plus the `_colon_body` forms (`#show` term / `#project` atom / `#external`)
        // whose `: body` is a real, breakable rule body routed through the trailing-neck
        // shape — the `:` is already a `Connective` opener in `directive_parts`.
        || id == k.show
        || id == k.show_signature
        || id == k.show_term
        || id == k.defined
        || id == k.project_signature
        || id == k.project_atom
        || id == k.const_
        || id == k.include
        || id == k.external
        || id == k.signature
        // `#program name(params).` (bespoke `lower_program`, bracketed path) and its
        // `parameters` `,`-list (a separated sequence); `#heuristic atom : body. [w, t]`
        // (the generic colon-body directive) and the `weight` (`term @ priority`) inside
        // its `[ … ]` tail.
        || id == k.program
        || id == k.heuristic
        || id == k.parameters
        || id == k.weight
        // `#edge ( pair (; pair)* ) (: body)? .` (bespoke `lower_edge`, bracketed /
        // trailing-neck path) and its `edge_pair` (`term, term`) operand.
        || id == k.edge
        || id == k.edge_pair
        // `:~ body. [ weight (, terms)? ]` (the weak constraint — `statement_parts`
        // shares the leading-neck shape, gobbling the `[…]` tail) and the embedded
        // `#script (lang) code #end.` (bespoke `lower_script`, the `code` body verbatim).
        || id == k.weak_constraint
        || id == k.script
}

/// The conditional-literal `… : condition` family — one shape, one lowering
/// ([`lower_conditional`], which documents each kind's prefix): a plain
/// `conditional_literal`, the set / body / head aggregate elements, the
/// `optimize_element`, and the `theory_element`.
fn is_conditional_family(k: &crate::cst::KindIds, id: u16) -> bool {
    id == k.conditional_literal
        || id == k.set_aggregate_element
        || id == k.body_aggregate_element
        || id == k.head_aggregate_element
        || id == k.optimize_element
        || id == k.theory_element
}

/// Build the Doc for a handled composite from its already-lowered children. The
/// rule looks each child's Doc up in `built` by `child.id()` (no recursion),
/// assigns [`super::assemble::Role`]s, and calls the assembler; `bracket_depth` is
/// the node's own depth, which the depth-sensitive spacing consults. Returns
/// `None` for an unhandled kind (→ the spine's verbatim default).
///
/// INVARIANT: a kind admitted by [`handles`] should be fully built here. Reserve
/// `None` for genuine anomalies (the totality backstop), never as ordinary control
/// flow: a node the spine DESCENDED into has had every child lowered already, so a
/// late `None` discards that work and re-emits the whole node verbatim and
/// UN-formatted. The only sanctioned `None`s are an unhandled kind (the `else`,
/// unreachable given [`handles`]) and the `has_unformattable_child` comment / ERROR
/// degrade — both byte-exact verbatim, so totality and `≈` still hold.
pub(super) fn build(
    ctx: &Ctx,
    node: Node,
    bracket_depth: usize,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> Option<NodeId> {
    let k = ctx.kinds;
    let id = node.kind_id();
    if id == k.source_file {
        return Some(lower_source_file(ctx, node, built, b));
    }
    // Every other handled construct degrades to its verbatim source span on an ERROR /
    // MISSING / pool-edge child (the sanctioned totality backstop); guard once for the whole
    // dispatch table rather than per arm. Comments do NOT degrade — leading / trailing are
    // re-injected at their anchors and dangling via `push_dangling` (see
    // `has_unformattable_child`).
    if has_unformattable_child(ctx, node) {
        return None;
    }
    // `bracket_depth` is the node's own depth, read by the separated-sequence and
    // conditional-family arms for depth-based comma / `;` tightening (`comma_spacing`).
    if id == k.rule || id == k.integrity_constraint || id == k.weak_constraint {
        // A headed rule, an integrity constraint, or a weak constraint — all the
        // statement shape (`statement_parts` + the assembler's neck logic). The weak
        // constraint adds a leading `:~` neck and a gobbled `[ weight (, terms)? ]` tail.
        Some(lower_statement(ctx, node, built, b))
    } else if id == k.set_aggregate
        || id == k.head_aggregate
        || id == k.body_aggregate
        || id == k.theory_atom
        || id == k.minimize
        || id == k.maximize
    {
        // A bracketed aggregate: an optional bound, a `{`-block of
        // `;`-elements, a dedented `}`, an optional bound or statement-`.` tail.
        // `minimize` / `maximize` are top-level statements that own their dot;
        // the choice / head / body aggregates and `theory_atom` are atoms (a head,
        // or a `body_literal`'s atom). `body_aggregate` shares the head shape, so
        // the same role assignment serves.
        Some(lower_aggregate(ctx, node, built, b))
    } else if id == k.body
        || id == k.disjunction
        || id == k.condition
        || id == k.set_aggregate_elements
        || id == k.head_aggregate_elements
        || id == k.body_aggregate_elements
        || id == k.theory_elements
        || id == k.optimize_elements
        || id == k.terms
        || id == k.theory_terms
        || id == k.theory_operator_definitions
        || id == k.parameters
    {
        // A rule body (`,`/`;`-joined literals), a disjunctive head (`;`/`,`/`|`-
        // joined, separators preserved), a conditional literal's `condition`
        // (`,`-joined literals), the aggregate element lists (`;`-joined), a
        // `,`-list argument / tuple-item node (`terms`), and the `#program`
        // `parameters` (`id, id, …`) are all the same separated-sequence shape.
        Some(lower_separated_sequence(ctx, node, bracket_depth, built, b))
    } else if id == k.body_literal || id == k.literal {
        // A signed atom: an optional sign (`not` / `not not`, spaced) on an atom.
        // Shared by body literals and head / disjunct / condition `literal`s (same
        // grammar shape — optional `sign` + `atom`).
        Some(lower_signed_atom(ctx, node, built, b))
    } else if is_conditional_family(k, id) {
        // The conditional-literal `… : condition` family — one shape, one lowering
        // (`lower_conditional` documents each kind's prefix). The first `:`
        // is the neck and the condition hangs; `bracket_depth` sets the prefix comma's
        // spacing.
        Some(lower_conditional(ctx, node, bracket_depth, built, b))
    } else if id == k.comparison {
        // A comparison is a flat `term (relation term)+` with each relation spaced.
        Some(lower_comparison(ctx, node, built, b))
    } else if id == k.lower || id == k.upper {
        // An aggregate bound — `lower` (`term relation?`) / `upper` (`relation? term`).
        // The optional relation is spaced from its term and normalized, like a
        // comparison; the outer space against the brace / function is `aggregate_parts`'.
        Some(lower_guard(ctx, node, built, b))
    } else if id == k.symbolic_atom
        || id == k.function
        || id == k.external_function
        || id == k.tuple
        || id == k.theory_function
        || id == k.theory_tuple
        || id == k.theory_list
        || id == k.theory_set
    {
        // An applicative term/atom — `[-]name(args)` / `@name(args)` — or a
        // `tuple` (`(items)`, empty name). A bracketed construct: the name abuts `(`,
        // the `,`-list arguments ride between the parens (Pad::Tight), `;` separates
        // argument-pool / tuple-pool segments. This is also reused for the theory
        // bracketed terms (`theory_function`/`theory_tuple` `(…)`, `theory_list` `[…]`,
        // `theory_set` `{…}`) — same shape, the opener recognition spanning all three
        // bracket pairs; theory terms carry no pool `;`, so that arm stays dormant.
        Some(lower_application(ctx, node, bracket_depth, built, b))
    } else if id == k.binary_operation {
        // A binary term operation (`+ - * / \ ** ^ ? & ..`), FLAT in v0.
        Some(lower_binary_operation(ctx, node, bracket_depth, built, b))
    } else if id == k.unary_operation {
        // A unary term operation (`-` / `~`), always tight, FLAT.
        Some(lower_unary_operation(ctx, node, built, b))
    } else if id == k.abs {
        // An abs term `|t|` / `|t; …|` — a tight bracket-pair (`|`…`|`).
        Some(lower_abs(ctx, node, bracket_depth, built, b))
    } else if id == k.theory_unparsed_term {
        // A theory operator-application term — always-spaced operators, FLAT.
        Some(lower_theory_unparsed_term(ctx, node, built, b))
    } else if id == k.theory_operators {
        // A run of theory operators — space-separated (`+ +` ≠ `++`), or the
        // comma-separated atom-definition guard list (`{<=, >=}`).
        Some(lower_theory_operators(ctx, node, built, b))
    } else if id == k.theory_atom_upper {
        // A theory atom's `op term` upper bound (the `right` field).
        Some(lower_theory_atom_upper(ctx, node, built, b))
    } else if id == k.theory {
        // The `#theory name { defs } .` definition directive.
        Some(lower_theory_directive(ctx, node, built, b))
    } else if id == k.theory_term_definition {
        // A `name { operator_definitions }` term definition.
        Some(lower_theory_term_definition(ctx, node, built, b))
    } else if id == k.theory_operator_definition {
        // An `op : priority, arity[, assoc]` operator definition.
        Some(lower_theory_operator_definition(ctx, node, built, b))
    } else if id == k.theory_atom_definition {
        // An `&name/arity : term_name, [{ops}, guard,] atom_type` definition.
        Some(lower_theory_atom_definition(ctx, node, built, b))
    } else if id == k.show
        || id == k.show_signature
        || id == k.show_term
        || id == k.defined
        || id == k.project_signature
        || id == k.project_atom
        || id == k.const_
        || id == k.include
        || id == k.external
        || id == k.heuristic
    {
        // A keyword + operand(s) [+ `[…]` tail] directive through the generic
        // role-assigner. Two shapes, ONE lowering: the flat (no `:` body) forms — `#show`
        // / `#defined` / `#project` (signature), `#const` (`=`-binding + optional `[type]`),
        // `#include` (string / `<id>`) — and the `_colon_body` forms `#show` term /
        // `#project` atom / `#external` (the last with an optional `[type]` tail), whose
        // `: body` `directive_parts` roles as a `Connective` opener so `assemble` routes it
        // through the SAME trailing-neck shape as a conditional literal (head `#external
        // p(X)`, neck `:`, the breakable body drops to indent, tail `.`). A colon-less form
        // carries no connective and falls to the flat separated shape. `#heuristic atom :
        // body. [weight, type]` is another colon-body form: its mandatory `[ … ]` tail is
        // gobbled by `bracket_tail` (the `weight` inside it rides as one Element, lowered
        // by `lower_flat_tight`).
        Some(lower_directive(ctx, node, built, b))
    } else if id == k.signature {
        // A `signature` (`[-]name / arity`) — `/` tight, FLAT.
        Some(lower_flat_tight(ctx, node, built, b))
    } else if id == k.program {
        // `#program name(params).` — bespoke, routed through the bracketed path
        // (the `(params)` `,`-list needs real separator spacing the generic can't give).
        Some(lower_program(ctx, node, built, b))
    } else if id == k.weight {
        // A `weight` (`term @ priority`) inside a `[ … ]` tail — `@` tight, FLAT.
        Some(lower_flat_tight(ctx, node, built, b))
    } else if id == k.edge {
        // `#edge ( pair (; pair)* ) (: body)? .` — bespoke (the `(pairs)`
        // separators need real spacing the generic can't give); routed bracketed, or
        // trailing-neck when a `: body` is present (see `lower_edge`).
        Some(lower_edge(ctx, node, built, b))
    } else if id == k.edge_pair {
        // An `edge_pair` (`term, term`) — one FLAT Element inside `lower_edge`.
        Some(lower_edge_pair(ctx, node, built, b))
    } else if id == k.script {
        // `#script (lang) code #end.` — bespoke, FLAT; the `code` body is a
        // verbatim leaf the spine emitted, preserved byte-for-byte.
        Some(lower_script(ctx, node, built, b))
    } else {
        None
    }
}

/// The translation unit: statements on consecutive lines (a `hardline` between them,
/// never grouped) with blank-line normalization — ONE author blank is preserved
/// between two items when the source put ≥1 blank line anywhere in their gap (including
/// around a re-injected comment, via `carry_blank`), and one blank is forced before a
/// `#program` section. Detection is off tree-sitter SOURCE rows (the signal the comment
/// classifier uses). Leading / trailing file blanks and the single terminating newline
/// are handled by the renderer + the `format` post-pass.
fn lower_source_file(
    ctx: &Ctx,
    node: Node,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    // The root's DANGLING comments are the standalone / section comments: those after
    // the final statement (a comment-only file, a trailing comment past the last `.`)
    // AND those a blank line detaches from the statement below them. They must be
    // emitted at their SOURCE POSITION among the statements, NOT all relocated to the
    // bottom: a detached top-of-file comment that jumps to the end reorders the comment
    // sequence and breaks `≈`. Cloned into an owned list so the walk closure holds no
    // borrow of `ctx.at` (`dangling` is tiny).
    let dangling = ctx
        .at
        .for_node(node.id())
        .map(|ac| ac.dangling.clone())
        .unwrap_or_default();
    let mut items = Vec::new();
    // `prev_end_row` = the SOURCE end row of the previous emitted-or-skipped child;
    // `carry_blank` = a blank seen before a skipped (re-injected) comment, owed to the
    // item that comment rides with. A gap of ≥2 rows between consecutive children is one
    // author blank line (the source's grouping signal).
    let mut prev_end_row: Option<usize> = None;
    let mut carry_blank = false;
    crate::cst::walk_children(node, |_field, child| {
        let start_row = child.start_position().row;
        let gap_blank = prev_end_row.is_some_and(|p| start_row >= p + 2);
        prev_end_row = Some(child.end_position().row);
        if super::reinject::is_comment(ctx.kinds, child) {
            // A leading / trailing comment is re-injected via its statement's `child_doc`
            // (skipped here, but its blank is carried forward); a blank-DETACHED root
            // comment is in `dangling` and is emitted standalone at this source position.
            let range = child.byte_range();
            match dangling.iter().find(|t| t.range == range) {
                Some(t) => {
                    let doc = super::reinject::comment_doc(ctx.src, t, b);
                    push_item(b, &mut items, doc, gap_blank || carry_blank);
                    carry_blank = false;
                }
                None => carry_blank |= gap_blank,
            }
        } else {
            let doc = child_doc(ctx, built, child, b);
            let is_program = child.kind_id() == ctx.kinds.program;
            push_item(b, &mut items, doc, gap_blank || carry_blank || is_program);
            carry_blank = false;
        }
    });
    b.seq(&items)
}

/// Push one `source_file` item, preceded — when it is not the first — by the
/// inter-statement break: a plain `hardline`, plus a `blank_line` when one author blank
/// is preserved. The two coalesce in the renderer to exactly one blank line,
/// even across a re-injected comment's own trailing / leading hardline.
fn push_item(b: &mut DocBuilder, items: &mut Vec<NodeId>, doc: NodeId, blank: bool) {
    if !items.is_empty() {
        let nl = b.hardline();
        items.push(nl);
        if blank {
            let bl = b.blank_line();
            items.push(bl);
        }
    }
    items.push(doc);
}

/// A headed rule (`head :- body.`) or an integrity constraint (`:- body.`). Both
/// reduce to the same role assignment; the assembler's neck-position logic does the
/// rest — a `:-` at position 0 leads (constraint), elsewhere it trails (rule).
fn lower_statement(
    ctx: &Ctx,
    node: Node,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    let (mut parts, neck_tail_fuses) = statement_parts(ctx, node, built, b);
    let (indent, neck) = nest_deltas(ctx.style);
    super::reinject::push_dangling(ctx, node, &mut parts, b);
    assemble(b, &parts, indent, neck, neck_tail_fuses)
}

/// A pure separated sequence — a rule body (`,` / `;`-joined literals) or a
/// disjunctive head (`;` / `,` / `|`-joined, separators preserved as authored).
/// Routed through the assembler's separated-sequence branch, which wraps it in its
/// OWN group, so under a broken neck it stays flat if it fits (recursive-minimal).
/// It carries no opener, so the assembler's `indent` / `neck` are inert here.
fn lower_separated_sequence(
    ctx: &Ctx,
    node: Node,
    bracket_depth: usize,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    let k = ctx.kinds;
    let mut parts = Vec::new();
    // Preserve ONE author blank between exploded elements. This is the choke point
    // for the `,`/`;`-separated sequences the dispatch routes here — rule bodies,
    // disjunctions, every aggregate element list, conditions, comma argument-lists / tuple
    // items (a `terms` node), theory element / term lists, optimize lists, and `#program`
    // parameters. (Term POOLS — `f(a; b)`, abs `|a; b|` — separate their `;` at the
    // application / abs level, not here, so a blank *inside a pool* is not preserved:
    // vanishingly rare and `≈`-safe, a documented v0 limit.)
    let mut blanks = ElementBlanks::new();
    // Walk ALL children (not `walk_code_children`): re-injected comments are not parts here,
    // but they occupy source rows, so the blank tracker must see them (a comment line
    // between a separator and its element must NOT read as an author blank — the bug a
    // code-only walk has). ERROR children never reach here: an unformattable body degrades
    // to verbatim upstream (`has_unformattable_child`).
    crate::cst::walk_children(node, |_field, child| {
        // A re-injected leading / trailing comment: skip as a part (the element's
        // `child_doc` / the node's `push_dangling` re-injects it), but thread its rows.
        if super::reinject::is_comment(ctx.kinds, child) {
            blanks.note(child);
            return;
        }
        // The named children are the literals (Element); the anonymous `,` / `;` / `|` are
        // the separators. (Bracketed constructs, which also carry anonymous openers /
        // closers, classify those by kind — a later family; the body has none.)
        if child.is_named() {
            parts.push(Part {
                role: Role::Element,
                doc: element_doc(ctx, built, child, &mut blanks, b),
            });
        } else {
            // `,` / `;` hug the left element (trailing) and tighten with bracket depth
            // (`comma_spacing`: Spaced at depth 0–1, Tight from depth 2; `;` shares the
            // threshold). The disjunction `|` is an alternation bar, spaced on BOTH
            // sides — always `Spaced`, with a baked LEADING space (the trailing
            // break point follows every separator alike). This is the only `|`-bearing
            // separated sequence (abs-pool `|…|` is its own family).
            // A separator anchors / carries the blank for the element that follows it.
            blanks.note(child);
            let sep = child_doc(ctx, built, child, b);
            let (sep_kind, doc) = if child.kind_id() == k.pipe {
                let space = b.space();
                (Sep::Spaced, b.seq(&[space, sep]))
            } else {
                (comma_spacing(bracket_depth), sep)
            };
            parts.push(Part {
                role: Role::Separator(sep_kind),
                doc,
            });
        }
    });
    // A trailing separator (the semantic 1-tuple comma `(a,)`, or a multi-tuple
    // trailing comma `(a, b,)`) has no following element — retag it as a glued
    // `Tail` so it hugs the last element instead of dangling a break point (the
    // `separated` precondition is infix separators). Only a tuple `terms` produces a
    // trailing separator; bodies / disjunctions / aggregate lists never do, so this
    // is a no-op for them. NEVER synthesize one; only preserve.
    if let Some(last) = parts.last_mut() {
        if matches!(last.role, Role::Separator(_)) {
            last.role = Role::Tail;
        }
    }
    let (indent, neck) = nest_deltas(ctx.style);
    super::reinject::push_dangling(ctx, node, &mut parts, b);
    assemble(b, &parts, indent, neck, false)
}

/// A bracketed aggregate: the choice `set_aggregate`, the optimization
/// `minimize` / `maximize`, the `head_aggregate`, the `theory_atom`. All share one
/// shape — an optional bound, a `{`-opened block of `;`-separated elements, a
/// dedented `}`, an optional bound or `.` tail — so one role assignment
/// ([`aggregate_parts`]) feeds the assembler's bracketed path.
fn lower_aggregate(
    ctx: &Ctx,
    node: Node,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    let mut parts = aggregate_parts(ctx, node, built, b);
    let (indent, neck) = nest_deltas(ctx.style);
    super::reinject::push_dangling(ctx, node, &mut parts, b);
    assemble(b, &parts, indent, neck, false)
}

/// An applicative bracketed construct — `symbolic_atom` (`[-]name(args)`),
/// `function` (`name(args)`), and `external_function` (`@name(args)`); reused by
/// `tuple` (`(items)`, empty name), and by the theory bracketed terms
/// `theory_function` / `theory_tuple` (`(…)`), `theory_list` (`[…]`), and `theory_set`
/// (`{…}`) — all `Pad::Tight` term brackets, the opener/closer recognition spanning the
/// three pairs. (`abs` `|items|` uses a sibling rule — its `|` opener needs first/last
/// disambiguation.) The grammar shape is uniform:
/// optional name parts, a bracket opener, a body of `,`-list argument segments
/// separated by pool `;`, a closer. Routed through `assemble`'s bracketed path — the
/// name abuts the opener; the body explodes one segment per line with the `;`
/// trailing, or stays flat hugging the parens (`Pad::Tight`, "none inside term
/// parens"). Pool `;` and the arguments sit ONE bracket deeper than this node, so the
/// `;` spacing reads `bracket_depth + 1` (`comma_spacing`); the `,`-list interiors
/// re-enter that same depth via the `arguments` field (`child_depth`). The pool `;`
/// is `HardPunct` on both sides — non-fusing by construction (`fusion` arm [5]) — so
/// no per-seam assert is owed (the operator seams are where one is).
fn lower_application(
    ctx: &Ctx,
    node: Node,
    bracket_depth: usize,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    let k = ctx.kinds;
    let mut parts = Vec::new();
    super::reinject::walk_code_children(ctx.kinds, node, |_field, child| {
        let id = child.kind_id();
        let doc = child_doc(ctx, built, child, b);
        let role = if id == k.lparen || id == k.lbracket || id == k.lbrace {
            // `(` for applications / tuples; `[` / `{` for theory lists / sets.
            // All term-level brackets hug their interior (`Pad::Tight`).
            Role::Opener(OpenerKind::Bracket(Pad::Tight))
        } else if id == k.rparen || id == k.rbracket || id == k.rbrace {
            Role::Closer
        } else if id == k.semicolon {
            // A pool `;` sits one bracket deeper than this node (inside the parens):
            // the SAME structural fact `child_depth` (mod.rs) threads onto the
            // descended `arguments` `terms`, computed here in-rule because the `;` is a
            // direct child, not a descended node. Keep the two `+1`s in sync.
            Role::Separator(comma_spacing(bracket_depth + 1))
        } else {
            // Name parts (`-`, the identifier, `@`) before the opener, and the
            // `arguments` segment(s) (each an already-lowered `terms`) between them.
            Role::Element
        };
        parts.push(Part { role, doc });
    });
    let (indent, neck) = nest_deltas(ctx.style);
    super::reinject::push_dangling(ctx, node, &mut parts, b);
    assemble(b, &parts, indent, neck, false)
}

/// An absolute-value term `|t|` / a pool `|t; …|` (grammar: `seq("|", term,
/// repeat(seq(";", term)), "|")`). A tight bracket-pair: the first `|` is a
/// `Pad::Tight` opener, the last `|` its closer (dispatch on the `abs` NODE kind, not
/// the bare `|`, which is also the disjunction separator); the inner terms form a
/// `;`-pool (depth+1) between them. `|X|` hugs (`Pad::Tight`); `|a; b|` follows the
/// list rule. Both `|`s are `HardPunct`, so no per-seam assert is owed.
fn lower_abs(
    ctx: &Ctx,
    node: Node,
    bracket_depth: usize,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    let k = ctx.kinds;
    let mut opener_seen = false;
    let mut parts = Vec::new();
    super::reinject::walk_code_children(ctx.kinds, node, |_field, child| {
        let doc = child_doc(ctx, built, child, b);
        let role = if child.kind_id() == k.pipe {
            // The first `|` opens, the second (last) closes.
            if opener_seen {
                Role::Closer
            } else {
                opener_seen = true;
                Role::Opener(OpenerKind::Bracket(Pad::Tight))
            }
        } else if child.kind_id() == k.semicolon {
            // The abs pool `;` is one bracket deep (inside the `|…|`); mirrors
            // `child_depth`'s +1 for the descended interior terms (see `lower_application`).
            Role::Separator(comma_spacing(bracket_depth + 1))
        } else {
            Role::Element
        };
        parts.push(Part { role, doc });
    });
    let (indent, neck) = nest_deltas(ctx.style);
    super::reinject::push_dangling(ctx, node, &mut parts, b);
    assemble(b, &parts, indent, neck, false)
}

/// A theory operator-application term — `[root]? (operators root)+` (grammar
/// `theory_unparsed_term`). Every `theory_operator` is rendered with SURROUNDING
/// SPACES: two adjacent theory operators are a greedy maximal-munch hazard
/// (`+ +` ≠ `++`), so unlike the fixed-length term operators they are NEVER
/// tightened — the one place arithmetic-tightening does not apply. Shipped FLAT (a
/// bare `seq`, no group); the push-time forced-break fold keeps a deep chain
/// stack-safe. This rule spaces EVERY operator, so it creates no tight theory seam
/// at all — no per-seam assert is owed here. (The only theory-op-vs-token abutments
/// are the `#theory` guard set's `{ ops }` braces and commas — see
/// `lower_theory_atom_definition`, where the structural ≈-safety argument lives.)
fn lower_theory_unparsed_term(
    ctx: &Ctx,
    node: Node,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    // Join every child (root terms and operator runs alike) with a single space: every
    // gap in a `theory_unparsed_term` is adjacent to a `theory_operators` run, and theory
    // operators are ALWAYS spaced — `theory_operator_spacing()` is `Spaced` by
    // construction (Spaced is the only `≈`-sound value: abutting a theory operator could
    // munch it across the seam). The space is therefore unconditional.
    debug_assert_eq!(theory_operator_spacing(), Sep::Spaced);
    let mut items = Vec::new();
    super::reinject::walk_code_children(ctx.kinds, node, |_field, child| {
        if !items.is_empty() {
            let sp = b.space();
            items.push(sp);
        }
        items.push(child_doc(ctx, built, child, b));
    });
    b.seq(&items)
}

/// A `theory_operators` node. The grammar aliases two shapes to this one kind: the
/// `repeat1(_theory_operator)` RUN inside a `theory_unparsed_term` (adjacent operators,
/// no commas — e.g. `+ +`), and the COMMA-separated guard list inside a
/// `theory_atom_definition` (`{<=, >=}`). Both render the same way under one rule:
/// adjacent operators are space-separated so a run like `+ +` never fuses to `++`
/// (greedy-munch carve-out), while a `,` hugs the left operator and is followed by a
/// space (`<=, >=`). A single-operator run (the common case, `-` in `x - y`) emits just
/// the operator.
fn lower_theory_operators(
    ctx: &Ctx,
    node: Node,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    // Theory operators are ALWAYS spaced — `theory_operator_spacing()` is `Spaced` by
    // construction (the only `≈`-sound value); the inter-operator space below is
    // therefore unconditional.
    debug_assert_eq!(theory_operator_spacing(), Sep::Spaced);
    let k = ctx.kinds;
    let mut items = Vec::new();
    let mut need_space = false;
    super::reinject::walk_code_children(ctx.kinds, node, |_field, child| {
        let doc = child_doc(ctx, built, child, b);
        if child.kind_id() == k.comma {
            // A comma-separated guard list: the `,` hugs the left operator, a space
            // follows; the next operator then needs no leading space.
            let sp = b.space();
            items.extend_from_slice(&[doc, sp]);
            need_space = false;
        } else {
            if need_space {
                let sp = b.space();
                items.push(sp);
            }
            items.push(doc);
            need_space = true;
        }
    });
    b.seq(&items)
}

/// A theory atom's upper bound `op term` (grammar `theory_atom_upper`, the atom's
/// `right` field — e.g. `<= 3`, `= x + y`). The leading `_theory_operator` is rendered
/// with a trailing space before its `theory_term` (always-spaced); the space
/// BEFORE the operator is supplied by the `}` closer's `right` tail in `aggregate_parts`
/// (`} <= 3`). FLAT.
fn lower_theory_atom_upper(
    ctx: &Ctx,
    node: Node,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    // The operator is always spaced from its term (`theory_operator_spacing()` is
    // `Spaced` by construction — the only `≈`-sound value); space unconditionally.
    debug_assert_eq!(theory_operator_spacing(), Sep::Spaced);
    let mut items = Vec::new();
    super::reinject::walk_code_children(ctx.kinds, node, |_field, child| {
        if !items.is_empty() {
            let sp = b.space();
            items.push(sp);
        }
        items.push(child_doc(ctx, built, child, b));
    });
    b.seq(&items)
}

/// The `#theory name { defs } .` definition directive. `#theory` is a directive
/// keyword spaced from its name; the block `{` is a `Pad::Spaced` bracket opener
/// trailing the name (`#theory t {`); the `;`-separated definitions are direct children
/// (the `_theory_definitions` rule is inlined), explode one per line, and the `.` glues
/// to the dedented closer (`}.`). The `;` separators are `Sep::Spaced`: the `#theory`
/// brace does NOT bump `bracket_depth` (its definitions carry field separators, not
/// nested-argument commas — they must never tighten), so the `;` stays spaced;
/// indentation still composes via the bracket opener's `nest`. A `seen_opener` flag
/// distinguishes the name parts (before the `{`, spaced) from the body definitions
/// (after it, hugged by the `;`).
fn lower_theory_directive(
    ctx: &Ctx,
    node: Node,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    let k = ctx.kinds;
    let mut parts = Vec::new();
    let mut seen_opener = false;
    super::reinject::walk_code_children(ctx.kinds, node, |_field, child| {
        let id = child.kind_id();
        let doc = child_doc(ctx, built, child, b);
        let (role, doc) = if id == k.lbrace {
            seen_opener = true;
            (Role::Opener(OpenerKind::Bracket(Pad::Spaced)), doc)
        } else if id == k.rbrace {
            (Role::Closer, doc)
        } else if id == k.dot {
            (Role::Tail, doc)
        } else if id == k.semicolon {
            (Role::Separator(Sep::Spaced), doc)
        } else if seen_opener {
            // A definition in the `{ … }` body — no injected space (the `;` separates).
            (Role::Element, doc)
        } else {
            // The `#theory` keyword or the name, before the `{` — spaced from what follows.
            let sp = b.space();
            (Role::Element, b.seq(&[doc, sp]))
        };
        parts.push(Part { role, doc });
    });
    let (indent, neck) = nest_deltas(ctx.style);
    super::reinject::push_dangling(ctx, node, &mut parts, b);
    assemble(b, &parts, indent, neck, false)
}

/// A `theory_term_definition` — `name { operator_definitions }`. The name leads a
/// `Pad::Spaced` block (`d { … }`) whose `;`-separated operator definitions explode one
/// per line; the `operator_definitions` body rides as a single Element (its own group).
fn lower_theory_term_definition(
    ctx: &Ctx,
    node: Node,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    let k = ctx.kinds;
    let mut parts = Vec::new();
    let mut seen_opener = false;
    super::reinject::walk_code_children(ctx.kinds, node, |_field, child| {
        let id = child.kind_id();
        let doc = child_doc(ctx, built, child, b);
        let (role, doc) = if id == k.lbrace {
            seen_opener = true;
            (Role::Opener(OpenerKind::Bracket(Pad::Spaced)), doc)
        } else if id == k.rbrace {
            (Role::Closer, doc)
        } else if seen_opener {
            (Role::Element, doc) // the operator-definitions body
        } else {
            let sp = b.space(); // the name, before the `{`
            (Role::Element, b.seq(&[doc, sp]))
        };
        parts.push(Part { role, doc });
    });
    let (indent, neck) = nest_deltas(ctx.style);
    super::reinject::push_dangling(ctx, node, &mut parts, b);
    assemble(b, &parts, indent, neck, false)
}

/// A `theory_operator_definition` — `operator : priority , arity [, associativity]`.
/// FLAT, all field separators always spaced: the operator (a `theory_operator`) then
/// ` : ` then the priority, then `, ` before the arity and (binary) associativity. The
/// `:` is spaced both sides; abutting `op:` is a hazard (`-:` greedy-munches into one
/// theory operator) the spacing forecloses (`fusion` arm [1]).
fn lower_theory_operator_definition(
    ctx: &Ctx,
    node: Node,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    let k = ctx.kinds;
    let mut items = Vec::new();
    super::reinject::walk_code_children(ctx.kinds, node, |_field, child| {
        let id = child.kind_id();
        let doc = child_doc(ctx, built, child, b);
        if id == k.colon {
            let (before, after) = (b.space(), b.space());
            items.extend_from_slice(&[before, doc, after]);
        } else if id == k.comma {
            // a field-separator comma: always spaced (`, `).
            let after = b.space();
            items.extend_from_slice(&[doc, after]);
        } else {
            // operator, priority, arity, associativity — each abuts the spaced separators.
            items.push(doc);
        }
    });
    b.seq(&items)
}

/// A `theory_atom_definition` — `& name / arity : term_name , [{ operators } , guard ,]
/// atom_type`. `&name/arity` is tight (the `&` abuts the name, the `/arity` is a
/// signature — both tight); the aliased `:` is spaced both sides; the `,` FIELD
/// separators (after `term_name` / `}` / guard) normalize to `, `. The optional guard
/// `{ operators }` is a `Pad::Tight` set whose comma-separated operators are spaced by
/// the descended `theory_operators` rule (so the guard's internal `,`s are NOT direct
/// children here — this walk sees only the field separators). FLAT.
///
/// The guard set is the ONE place a theory operator abuts another
/// token: `{`+`<=`, `>=`+`}`, and `<=`+`,` (the comma hugs its left operator). These are
/// `≈`-safe AND oracle-certified: `fusion::fuses` arm [1] is char-class-precise, and
/// `{` `}` `,` are NOT in the greedy theory-operator munch set (`fusion::is_theory_op_char`),
/// so `fuses` returns `false` for these seams (`{<=`→`{`,`<=`; `<=,`→`<=`,`,`). The emitter
/// therefore abuts here CONSISTENTLY with the oracle — not a carve-out. (A debug seam
/// tripwire à la `assert_operator_seam_safe` is now possible but low-value: this seam is
/// structurally invariant — the braces/comma are fixed tokens, nothing depth-varies.)
fn lower_theory_atom_definition(
    ctx: &Ctx,
    node: Node,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    let k = ctx.kinds;
    let mut items = Vec::new();
    super::reinject::walk_code_children(ctx.kinds, node, |_field, child| {
        let id = child.kind_id();
        let doc = child_doc(ctx, built, child, b);
        if id == k.colon {
            let (before, after) = (b.space(), b.space());
            items.extend_from_slice(&[before, doc, after]);
        } else if id == k.comma {
            let after = b.space();
            items.extend_from_slice(&[doc, after]);
        } else {
            // `&`, name, `/`, arity, term_name, the guard `{`/operators/`}`, guard name,
            // atom_type — each abuts the spaced separators.
            items.push(doc);
        }
    });
    b.seq(&items)
}

/// A binary term operation `left OP right` (`+ - * / \ ** ^ ? & ..`). v0 ships
/// operators FLAT (non-breaking): a bare `seq`, no group — the push-time
/// forced-break fold keeps a deep left-nested chain stack-safe. Spacing is
/// depth-based (`term_operator_spacing`: spaced at the top body level `X = Y + Z`,
/// tight inside any bracket `f(X, Y+Z)`), EXCEPT the interval `..`, which is ALWAYS
/// tight (a range *constructor*, not an arithmetic infix). RESERVED
/// SEAM: when chain-breaking lands, the continuation break LEADS here
/// (`binop_separator="Front"`), NEVER in `separated`. A Tight join runs the
/// debug-only seam tripwire — see [`assert_operator_seam_safe`].
fn lower_binary_operation(
    ctx: &Ctx,
    node: Node,
    bracket_depth: usize,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    // Dispatch on the grammar's own field labels (`field("left"/"operator"/"right")`,
    // grammar.js `binary_expression`) — the grammar's statement of each child's role
    // (POLS), like `aggregate_parts`'s `left`/`right`.
    let mut left = None;
    let mut op = None;
    let mut right = None;
    super::reinject::walk_code_children(ctx.kinds, node, |field, child| match field {
        Some("left") => left = Some(child),
        Some("operator") => op = Some(child),
        Some("right") => right = Some(child),
        _ => {}
    });
    let (Some(l), Some(o), Some(r)) = (left, op, right) else {
        // Not the expected `left OP right` shape — verbatim backstop (totality).
        return token::token(node, ctx.src, b);
    };
    let op_text = &ctx.src[o.byte_range()];
    let sep = if op_text == ".." {
        Sep::Tight
    } else {
        term_operator_spacing(bracket_depth)
    };
    let (l_doc, o_doc, r_doc) = (
        child_doc(ctx, built, l, b),
        child_doc(ctx, built, o, b),
        child_doc(ctx, built, r, b),
    );
    if sep == Sep::Tight {
        assert_operator_seam_safe(ctx, op_text, Some(l), Some(r));
        b.seq(&[l_doc, o_doc, r_doc])
    } else {
        let (before, after) = (b.space(), b.space());
        b.seq(&[l_doc, before, o_doc, after, r_doc])
    }
}

/// A unary term operation `OP operand` (`-` / `~`). Always tight (`-X` / `~X`),
/// FLAT. Dispatch on the grammar's field labels (`field("operator"/"right")`,
/// grammar.js `unary_expression`).
fn lower_unary_operation(
    ctx: &Ctx,
    node: Node,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    let mut op = None;
    let mut operand = None;
    super::reinject::walk_code_children(ctx.kinds, node, |field, child| match field {
        Some("operator") => op = Some(child),
        Some("right") => operand = Some(child),
        _ => {}
    });
    let (Some(o), Some(t)) = (op, operand) else {
        return token::token(node, ctx.src, b);
    };
    let op_text = &ctx.src[o.byte_range()];
    assert_operator_seam_safe(ctx, op_text, None, Some(t));
    let (o_doc, t_doc) = (child_doc(ctx, built, o, b), child_doc(ctx, built, t, b));
    b.seq(&[o_doc, t_doc])
}

/// Seam tripwire (`debug_assert`, so debug-only; **vacuous by design**). The op is
/// a fixed term operator (`Class::TermOp`); this checks it only against its WORD /
/// NUMBER operand boundaries (`tok_of` → `Ident` / `Number`). An operator- or
/// punctuation-boundary classes to `None` and is SKIPPED — and `fuses` is `false` for
/// every word/number-vs-`TermOp` pair anyway, so the check never fires on today's
/// grammar. That is the point: soundness rests NOT on this assert but on the
/// structural fact that the only fusing fixed term-op pair `*`+`*`=`**` is UNREACHABLE
/// (two binary ops never abut — an operand's boundary leaf is always a word / number /
/// `)` / `|`, never a bare operator). The assert is the cheap tripwire that fires if a
/// future change ever routes a genuinely-fusing pair through a Tight seam. The leaf
/// descent rides INSIDE `debug_assert!`, so it compiles out of release.
fn assert_operator_seam_safe(ctx: &Ctx, op_text: &str, left: Option<Node>, right: Option<Node>) {
    let op_tok = Tok {
        class: Class::TermOp,
        text: op_text,
    };
    debug_assert!(
        left.and_then(|n| tok_of(ctx, last_leaf(n)))
            .is_none_or(|lt| !fuses(lt, op_tok)),
        "term-op left seam re-lexes ({op_text})"
    );
    debug_assert!(
        right
            .and_then(|n| tok_of(ctx, first_leaf(n)))
            .is_none_or(|rt| !fuses(op_tok, rt)),
        "term-op right seam re-lexes ({op_text})"
    );
}

/// The leftmost leaf (token) of a subtree — the boundary token an operator abuts on
/// its left, for the seam assert. ITERATIVE (a `while` descent, not recursion) so
/// a deeply-nested operand can never overflow the stack (the iteration-over-recursion
/// discipline for KR-neutral depth walks).
fn first_leaf(mut node: Node) -> Node {
    while let Some(first) = node.child(0) {
        node = first;
    }
    node
}

/// The rightmost leaf (token) of a subtree — symmetric to [`first_leaf`], iterative.
fn last_leaf(mut node: Node) -> Node {
    loop {
        let count = node.child_count();
        if count == 0 {
            return node; // a leaf
        }
        let Some(last) = node.child(count - 1) else {
            return node; // unreachable (count > 0); keeps `last_leaf` total
        };
        node = last;
    }
}

/// A signed atom: an optional `sign` on an `atom`. Shared by `body_literal` and the
/// head / disjunct / condition `literal` (identical grammar shape). The default /
/// double negation (`not` / `not not`) is spaced from its atom (lest `not p`
/// fuse to `notp`); a sign-less literal is its atom unchanged. The atom rides on one
/// line (its own brackets, if any, bring their own group), so this is a bare
/// sequence, not a group. (A classical `-` is carried inside the `symbolic_atom`.)
fn lower_signed_atom(
    ctx: &Ctx,
    node: Node,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    let mut items = Vec::new();
    super::reinject::walk_code_children(ctx.kinds, node, |field, child| {
        items.push(child_doc(ctx, built, child, b));
        if field == Some("sign") && sign_spacing(&ctx.src[child.byte_range()]) == Sep::Spaced {
            let space = b.space();
            items.push(space);
        }
    });
    b.seq(&items)
}

/// A comparison: a flat `term (relation term)+`. Each `relation` (`<`, `<=`, `=`,
/// `!=`, …) is spaced on both sides (`X < Y`, chained `X < Y < Z`); the terms
/// are verbatim until a later family. It never breaks — a comparison is one body
/// literal, atomic on its line — so the spaces are mode-invariant.
fn lower_comparison(
    ctx: &Ctx,
    node: Node,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    let k = ctx.kinds;
    let mut items = Vec::new();
    super::reinject::walk_code_children(ctx.kinds, node, |_field, child| {
        let doc = child_doc(ctx, built, child, b);
        if child.kind_id() == k.relation {
            let before = b.space();
            let after = b.space();
            items.extend_from_slice(&[before, doc, after]);
        } else {
            items.push(doc);
        }
    });
    b.seq(&items)
}

/// An aggregate bound — `lower` (`term relation?`) or `upper` (`relation? term`).
/// Joins the bound's code children with a single space, so the optional `relation`
/// is spaced from its `term` and normalized regardless of the input (`{a}=1` →
/// `{ a } = 1`), matching a body comparison. Atomic — it never breaks; the outer
/// space against the aggregate brace / function is added by [`aggregate_parts`].
fn lower_guard(
    ctx: &Ctx,
    node: Node,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    let mut items = Vec::new();
    super::reinject::walk_code_children(ctx.kinds, node, |_field, child| {
        if !items.is_empty() {
            let sp = b.space();
            items.push(sp);
        }
        items.push(child_doc(ctx, built, child, b));
    });
    b.seq(&items)
}

/// A conditional literal `literal : condition`. The conditional `:` is a
/// connective opener that trails the literal, so [`super::assemble`] routes it
/// through the same trailing-neck shape as a headed rule: flat, the `:` is spaced
/// both sides (`b : c, d`); broken, the condition drops one further level
/// (indent 4) and hangs there. The `condition` is itself a `,`-joined `literal`
/// list (its own already-lowered separated sequence), so it rides as one Element.
/// A conditional literal with NO `condition` (`p :`, a valid error-free parse)
/// assembles over an empty body — `trailing_neck` then hangs nothing, yielding
/// `p :` (the `:` spaced before, never a trailing space, and never abutting a
/// following `-`/`~`).
fn lower_conditional(
    ctx: &Ctx,
    node: Node,
    bracket_depth: usize,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    let k = ctx.kinds;
    let mut parts = Vec::new();
    let mut seen_colon = false;
    super::reinject::walk_code_children(ctx.kinds, node, |_field, child| {
        let id = child.kind_id();
        let doc = child_doc(ctx, built, child, b);
        // The FIRST `:` is the connective opener: what follows hangs one level.
        // A LATER `:` — only in a `head_aggregate_element` `terms : literal : condition`
        // — is the guard separator: spaced both sides (a baked leading space + the
        // trailing break point `separated` adds) so it rides with the literal in the hung
        // body. `,` / `;` — an `optimize_element`'s `weight , terms` prefix — are
        // depth-spaced separators (so the prefix lays out spaced, not fused). Every other
        // child (the `literal` / `terms` / `weight` / `condition` content, each already
        // lowered with its own brackets) is an Element. Total: an unexpected child stays
        // an Element.
        let (role, doc) = if id == k.colon {
            if seen_colon {
                let sp = b.space();
                (Role::Separator(Sep::Spaced), b.seq(&[sp, doc]))
            } else {
                seen_colon = true;
                (Role::Opener(OpenerKind::Connective), doc)
            }
        } else if id == k.comma || id == k.semicolon {
            (Role::Separator(comma_spacing(bracket_depth)), doc)
        } else {
            (Role::Element, doc)
        };
        parts.push(Part { role, doc });
    });
    let (indent, neck) = nest_deltas(ctx.style);
    super::reinject::push_dangling(ctx, node, &mut parts, b);
    assemble(b, &parts, indent, neck, false)
}

/// Assign roles to a rule's / constraint's / weak-constraint's children: the
/// neck (`:-` or the weak `:~`) is the connective opener (the assembler makes it
/// lead or trail by position), the `.` is the tail, and the head and body are the
/// content elements. A weak constraint adds a `[ weight (, terms)? ]` tail after the
/// dot — gobbled by [`bracket_tail`] into one breakable, space-led `Tail` (so `b` is
/// threaded in for that one call; the rule / IC path has no `[…]` and never touches
/// it). Children are collected into `kids` first so the `[…]` gobble can advance the
/// index, mirroring [`directive_parts`].
fn statement_parts(
    ctx: &Ctx,
    node: Node,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> (Vec<Part>, bool) {
    let k = ctx.kinds;
    let mut kids: Vec<Node> = Vec::new();
    super::reinject::walk_code_children(ctx.kinds, node, |_field, child| kids.push(child));
    let mut parts = Vec::new();
    let mut neck_node = None;
    let mut tail_node = None;
    let mut i = 0;
    while i < kids.len() {
        let child = kids[i];
        let id = child.kind_id();
        if id == k.lbracket {
            // The weak-constraint `[ weight (, terms)? ]` tail: one breakable, space-led Tail.
            let (tail, next) = bracket_tail(ctx, &kids, i, built, b);
            parts.push(Part {
                role: Role::Tail,
                doc: tail,
            });
            i = next;
            continue;
        }
        let role = if id == k.neck || id == k.weak_neck {
            neck_node = Some(child);
            Role::Opener(OpenerKind::Connective)
        } else if id == k.dot {
            tail_node = Some(child);
            Role::Tail
        } else {
            Role::Element
        };
        parts.push(Part {
            role,
            doc: child_doc(ctx, built, child, b),
        });
        i += 1;
    }
    // The empty-body trailing-neck seam: does the neck fuse with the tail's first
    // token? Consumed only when the body is empty — a rule always has a body, so this is
    // dormant (`:-`/`.` are both HardPunct, non-fusing) and the belt for the tail
    // forms (theory `op term`, weak-constraint `[w@p]`).
    (parts, seam_fuses(ctx, neck_node, tail_node))
}

/// Assign roles to a bracketed aggregate's children. The `{` is the spaced
/// bracket opener and `}` its dedented closer; the element list rides between them
/// as the body. A `left` bound is spaced from what follows it (`1 {`, `1 <= #count{`);
/// a `right` bound rides the closer spaced (`} 1`, `} >= 2`); the statement `.` of
/// a `minimize` / `maximize` abuts the closer (`}.`). Every other child — the `&`,
/// the name, the parenthesized theory `arguments`, the `#count` / `#minimize`
/// keyword — is name content abutting the opener (`#count{`, `&p(1){`).
fn aggregate_parts(
    ctx: &Ctx,
    node: Node,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> Vec<Part> {
    let k = ctx.kinds;
    let mut parts = Vec::new();
    super::reinject::walk_code_children(ctx.kinds, node, |field, child| {
        let doc = child_doc(ctx, built, child, b);
        let id = child.kind_id();
        let (role, doc) = if field == Some("left") {
            // A lower bound is spaced from the brace / function it leads (`1 {`).
            let sp = b.space();
            (Role::Element, b.seq(&[doc, sp]))
        } else if field == Some("right") {
            // An upper bound rides the dedented closer, spaced (`} 1`, `} >= 2`).
            let sp = b.space();
            (Role::Tail, b.seq(&[sp, doc]))
        } else if id == k.lbrace {
            (Role::Opener(OpenerKind::Bracket(Pad::Spaced)), doc)
        } else if id == k.rbrace {
            (Role::Closer, doc)
        } else if id == k.dot {
            (Role::Tail, doc)
        } else {
            // Name content abutting the opener (`&`, name, theory args, the
            // `#count` / `#minimize` keyword) or the element-list body.
            (Role::Element, doc)
        };
        parts.push(Part { role, doc });
    });
    parts
}

/// Assign roles to a directive's children, for the ONE generic directive shape
/// fed to [`super::assemble`]. A directive *keyword* (`#show` / `#const`
/// / …) is the first child and is spaced from its operand (`#edge (` does not
/// abut), realized by baking a trailing space into its `Element` UNLESS the directive
/// is bare (`#show.`). The `_colon_body` `:` (`k.colon`) is a `Connective` opener, so a
/// `: body` directive routes through the SAME trailing-neck shape as a conditional
/// literal (head `#external p(X)`, neck `:`, body, tail `.`); a colon-less directive
/// has no connective and falls to the separated shape. The `=` (`#const`) is spaced both
/// sides; the `.` glues (`Tail`); a `[ … ]` tail is gobbled into one breakable `Tail`
/// ([`bracket_tail`]). `;`/`,` separators tighten by depth (`comma_spacing`); brackets
/// (`(`/`)` — `#program`/`#edge` route those through bespoke lowerings, never here).
fn directive_parts(
    ctx: &Ctx,
    node: Node,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> Vec<Part> {
    let k = ctx.kinds;
    // Collect children for one-step lookahead (the keyword-space peek, the `[ … ]`
    // gobble). O(children) scratch, O(n) total — the same pattern as `push_children`.
    let mut kids: Vec<Node> = Vec::new();
    super::reinject::walk_code_children(ctx.kinds, node, |_field, child| kids.push(child));
    let mut parts = Vec::new();
    let mut i = 0;
    while i < kids.len() {
        let child = kids[i];
        let id = child.kind_id();
        let doc = child_doc(ctx, built, child, b);
        if i == 0 {
            // The keyword: spaced from its operand unless the next child is the `.`.
            let bare = kids.get(1).is_some_and(|n| n.kind_id() == k.dot);
            let doc = if bare {
                doc
            } else {
                let sp = b.space();
                b.seq(&[doc, sp])
            };
            parts.push(Part {
                role: Role::Element,
                doc,
            });
        } else if id == k.colon {
            parts.push(Part {
                role: Role::Opener(OpenerKind::Connective),
                doc,
            });
        } else if id == k.equals {
            let (before, after) = (b.space(), b.space());
            let doc = b.seq(&[before, doc, after]);
            parts.push(Part {
                role: Role::Element,
                doc,
            });
        } else if id == k.dot {
            parts.push(Part {
                role: Role::Tail,
                doc,
            });
        } else if id == k.lbracket {
            // A `[ … ]` tail: gobble through the `]` into one breakable, space-led `Tail`.
            let (tail, next) = bracket_tail(ctx, &kids, i, built, b);
            parts.push(Part {
                role: Role::Tail,
                doc: tail,
            });
            i = next;
            continue;
        } else {
            // Operands (signature, atom, term, value, string, `<id>` tokens, body) ride
            // as `Element`s; their own Docs carry their internal layout. A `,`/`;` here
            // is a separator.
            //
            // INVARIANT: a directive routed through this generic must NOT carry a
            // top-level `,`/`;` separator together with a `:` body. `classify` puts the
            // `:` body's predecessors in the trailing-neck HEAD, which `assemble` lays by
            // `concat` (no separator breaks) — so a `Role::Separator` there would silently
            // lose its space/break (the `#edge` defect class). A directive that
            // needs both a top-level separator AND a `:` body must BAKE the separator as a
            // literal space (like `lower_edge`'s `;`) or use a bespoke lowering. On today's
            // grammar no generic directive does, so this arm is dormant-but-total.
            let role = if id == k.comma || id == k.semicolon {
                Role::Separator(comma_spacing(0))
            } else {
                Role::Element
            };
            parts.push(Part { role, doc });
        }
        i += 1;
    }
    // Executable form of the INVARIANT documented on the operand arm above: no
    // top-level `Separator` may precede the first `:` body (`Connective`), else it
    // would be lost in the trailing-neck head `concat` (the `#edge` defect
    // class). Compiled out of release builds.
    debug_assert!(
        {
            let connective = parts
                .iter()
                .position(|p| matches!(p.role, Role::Opener(OpenerKind::Connective)));
            match connective {
                Some(c) => !parts[..c]
                    .iter()
                    .any(|p| matches!(p.role, Role::Separator(_))),
                None => true,
            }
        },
        "directive_parts: a top-level Separator before a `:` body would be lost in the \
         trailing-neck head concat (see lower_edge); bake it as a literal space instead"
    );
    parts
}

/// Lower a directive through the generic role-assigner + the shared assembler.
fn lower_directive(
    ctx: &Ctx,
    node: Node,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    let mut parts = directive_parts(ctx, node, built, b);
    let (indent, neck) = nest_deltas(ctx.style);
    super::reinject::push_dangling(ctx, node, &mut parts, b);
    assemble(b, &parts, indent, neck, false)
}

/// `#program name (parameters)? .` — the keyword is spaced from the name; the name abuts
/// its `(` (an applicative opener, `Pad::Tight`); the `parameters` `,`-list rides as one
/// already-assembled Element between the parens; the `.` glues the closer. Routed through
/// the assembler's bracketed path (so empty `()` hugs and an over-long param list could
/// explode). Bespoke (not the generic) because the `(params)` separators need real
/// separator spacing, which only the inner assembled list provides.
///
/// Unlike `#edge` ([`lower_edge`]), `#program` has no `:` body, so its `(params)` are
/// never pulled into a trailing-neck head `concat`; the breakable `Role::Separator` path
/// inside the pre-assembled param Element is therefore safe here.
fn lower_program(
    ctx: &Ctx,
    node: Node,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    let k = ctx.kinds;
    let mut parts = Vec::new();
    let mut first = true;
    super::reinject::walk_code_children(ctx.kinds, node, |_field, child| {
        let id = child.kind_id();
        let doc = child_doc(ctx, built, child, b);
        let (role, doc) = if first {
            first = false;
            let sp = b.space(); // `#program ` keyword spaced from the name
            (Role::Element, b.seq(&[doc, sp]))
        } else if id == k.lparen {
            (Role::Opener(OpenerKind::Bracket(Pad::Tight)), doc)
        } else if id == k.rparen {
            (Role::Closer, doc)
        } else if id == k.dot {
            (Role::Tail, doc)
        } else {
            // the name, or the `parameters` `,`-list (one Element, its own group)
            (Role::Element, doc)
        };
        parts.push(Part { role, doc });
    });
    let (indent, neck) = nest_deltas(ctx.style);
    super::reinject::push_dangling(ctx, node, &mut parts, b);
    assemble(b, &parts, indent, neck, false)
}

/// Build a `[ … ]` tail — a weak-constraint `[w@p, terms]` or a directive
/// `#const n = v. [default]` / `#external p. [t]` / `#heuristic a : c. [w, t]` — as a real
/// breakable TIGHT bracket with a baked LEADING space. `kids[i]` is the `[`; returns
/// `(tail_doc, index-after-the-])`. The bracket rides the SAME `[`-opener machinery as every
/// other construct ([`super::assemble`]'s `bracketed`): it hugs flat (`[w, t]`, no interior
/// pad) when it fits and explodes one element per line with a dedented `]` when it
/// overflows. A `,` is a `Spaced` separator (the depth-0 `, ` / break point); the
/// bracket pair, the weight, the type/term, a `bool` / `const_type` leaf are all `Element`s.
/// A short tail fits flat — so a directive tail is unchanged — while a long weak-constraint
/// tail no longer drops its overflowing terms to column 0 (the compose-additively fix);
/// [`leading_neck`](super::assemble) nests the tail at the body hang so the explosion lands
/// at neck-width + indent with the closer dedenting to neck-width.
fn bracket_tail(
    ctx: &Ctx,
    kids: &[Node],
    i: usize,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> (NodeId, usize) {
    let k = ctx.kinds;
    let mut parts = Vec::new();
    let mut j = i;
    while j < kids.len() {
        let child = kids[j];
        let id = child.kind_id();
        let doc = child_doc(ctx, built, child, b);
        let role = if id == k.lbracket {
            Role::Opener(OpenerKind::Bracket(Pad::Tight))
        } else if id == k.rbracket {
            Role::Closer
        } else if id == k.comma {
            Role::Separator(comma_spacing(0))
        } else {
            Role::Element
        };
        parts.push(Part { role, doc });
        j += 1;
        if id == k.rbracket {
            break;
        }
    }
    let (indent, neck) = nest_deltas(ctx.style);
    let bracket = assemble(b, &parts, indent, neck, false);
    let lead = b.space();
    (b.seq(&[lead, bracket]), j)
}

/// Emit a node's children concatenated TIGHT — no inter-token spacing — so any source
/// whitespace normalizes away (the Doc model carries none). FLAT. Two callers:
/// `signature` (`[-]name / arity` → `p/1`, the `/` tight) and `weight` (`term @ priority`
/// → `2@1`, the `@` tight). The shared shape is "structural punctuation that must abut",
/// so one lowering serves both; the dispatch arms name which construct each is.
fn lower_flat_tight(
    ctx: &Ctx,
    node: Node,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    let mut items = Vec::new();
    super::reinject::walk_code_children(ctx.kinds, node, |_field, child| {
        items.push(child_doc(ctx, built, child, b));
    });
    b.seq(&items)
}

/// An `edge_pair` — `term , term` (`a, b`). The `,` is an UNCONDITIONAL baked literal
/// `, ` (the `,` Doc followed by a mode-invariant `b.space()`), exactly like
/// [`lower_edge`]'s pool `;`. `#edge` is ALWAYS top-level (depth 0) and its pairs are
/// deliberately always-flat, so the former depth-conditional `comma_spacing(0)` always
/// took the `Spaced` branch; baking the space unconditionally is behavior-identical and
/// makes the pair `,` uniform with the pool `;`. The terms are their own Docs.
/// FLAT, one Element inside [`lower_edge`]. The space is a LITERAL `b.space()` (not a break
/// point), so the pair never explodes and renders identically whether the enclosing `#edge`
/// is laid out bracketed or as a `concat`ed trailing-neck head (the classify interaction
/// documented on [`lower_edge`]).
fn lower_edge_pair(
    ctx: &Ctx,
    node: Node,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    let k = ctx.kinds;
    let mut items = Vec::new();
    super::reinject::walk_code_children(ctx.kinds, node, |_field, child| {
        let doc = child_doc(ctx, built, child, b);
        items.push(doc);
        if child.kind_id() == k.comma {
            // The pair `,` bakes a non-breaking `, ` (mode-invariant), uniform with
            // `lower_edge`'s pool `;`; `#edge` is top-level and its pairs are always-flat.
            let sp = b.space();
            items.push(sp);
        }
    });
    b.seq(&items)
}

/// `#edge ( pair (; pair)* ) (: body)? .` — the keyword is spaced from the `(`
/// (`#edge (` does not abut); the `;`-separated `edge_pair`s ride between the parens as the
/// bracketed body (`Pad::Tight`); an optional `: body` is a `Connective` after the `)`;
/// the `.` glues. Bespoke: the `(pairs)` need real separator spacing the generic forms
/// can't give.
///
/// Both separators are BAKED as literal (mode-invariant) spaces — the pair `,` inside
/// `lower_edge_pair`, the pool `;` here as a non-breaking `; ` Element (NOT a
/// `Role::Separator`). That is what makes the layout robust to `classify`: an
/// `#edge (…) : body.` carries BOTH a `Bracket` opener and a `Connective` opener, and
/// [`super::assemble`]'s `classify` finds the `:` FIRST, routing to a trailing-neck whose
/// head `#edge (…)` is laid out by `concat` (which copies the part docs but runs NO
/// `separated`, so a `Role::Separator` `;` would silently lose its space there — that was
/// the bug). The baked `; ` renders `#edge (a, b; c, d) : …` correctly on both the
/// bracketed and the neck paths. Consequence: edge pairs are ALWAYS laid flat (the explode
/// path was deemed low-value), keeping `#edge` spacing uniform with and
/// without a `: condition`; `≈`-safe either way, since only whitespace ever differs.
fn lower_edge(
    ctx: &Ctx,
    node: Node,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    let k = ctx.kinds;
    let mut parts = Vec::new();
    let mut first = true;
    super::reinject::walk_code_children(ctx.kinds, node, |_field, child| {
        let id = child.kind_id();
        let doc = child_doc(ctx, built, child, b);
        let (role, doc) = if first {
            first = false;
            let sp = b.space(); // `#edge ` keyword spaced from the `(`
            (Role::Element, b.seq(&[doc, sp]))
        } else if id == k.lparen {
            (Role::Opener(OpenerKind::Bracket(Pad::Tight)), doc)
        } else if id == k.rparen {
            (Role::Closer, doc)
        } else if id == k.semicolon {
            // The pool `;` is baked as a non-breaking `; ` (mode-invariant), so it renders
            // correctly whether the pairs land in the bracketed body OR in a trailing-neck
            // HEAD (which `concat`s without running `separated`). Consequence: edge pairs are
            // always laid flat — the explode path was deemed low-value, and this
            // keeps `#edge` spacing UNIFORM with and without a `: condition`.
            let sp = b.space();
            (Role::Element, b.seq(&[doc, sp]))
        } else if id == k.colon {
            (Role::Opener(OpenerKind::Connective), doc)
        } else if id == k.dot {
            (Role::Tail, doc)
        } else {
            // an `edge_pair` (one Element) or the `: body`
            (Role::Element, doc)
        };
        parts.push(Part { role, doc });
    });
    let (indent, neck) = nest_deltas(ctx.style);
    super::reinject::push_dangling(ctx, node, &mut parts, b);
    assemble(b, &parts, indent, neck, false)
}

/// `#script ( lang ) code #end .`. v0 formats the ASP wrapper — `#script (lang)`
/// (the keyword spaced from `(`; `lang` hugs the parens) and `#end.` (glued) — and
/// preserves the `code` body BYTE-FOR-BYTE (a `verbatim` leaf, emitted by the spine via
/// `token::token`). FLAT (a `#script` never explodes). The `code` is routed through
/// `embedded::format_embedded`, the identity in v0; the `debug_assert` pins that v0
/// contract (the hook returns the body unchanged). The owned-output completion (a new
/// owned-text Doc primitive + ruff/stylua) is a reserved seam.
fn lower_script(
    ctx: &Ctx,
    node: Node,
    built: &FxHashMap<usize, NodeId>,
    b: &mut DocBuilder,
) -> NodeId {
    let k = ctx.kinds;
    // Pin the v0 identity contract at the seam: the hook does not alter the body bytes.
    // `lang`/`code` are captured during the single emission walk below, then checked after.
    let mut lang = "";
    let mut code = "";
    let mut items = Vec::new();
    let mut first = true;
    super::reinject::walk_code_children(ctx.kinds, node, |field, child| {
        if field == Some("language") {
            lang = &ctx.src[child.byte_range()];
        } else if child.kind_id() == k.code {
            code = &ctx.src[child.byte_range()];
        }
        let doc = child_doc(ctx, built, child, b);
        if first {
            first = false; // `#script` keyword
            items.push(doc);
            let sp = b.space();
            items.push(sp);
        } else {
            // `(`, lang, `)`, the verbatim `code`, `#end`, `.` — all abut (the code
            // carries its own surrounding newlines, so `)` joins `\n…` and `…\n` joins
            // `#end`). No separator, no break: a flat sequence.
            items.push(doc);
        }
    });
    debug_assert_eq!(
        super::embedded::format_embedded(lang, code).as_ref(),
        code,
        "v0 format_embedded must be the identity"
    );
    b.seq(&items)
}

/// Classify a CST `node`'s token into its `fusion::Tok` for seam checks (kind-by-
/// NODE, never glyph). It covers the trailing-neck seam tokens — the `:-`/`:~` necks
/// (`HardPunct`), the conditional `:` (`Colon`), the statement `.` (`HardPunct`).
/// It also handles the operand word-token classes (`identifier`/`variable` → `Ident`,
/// `number` → `Number`) — the only operand-boundary leaves that could fuse with a
/// term operator. A boundary leaf this does not class (a `HardPunct` like `)` / `|`)
/// returns `None`, and BOTH `None`-callers stay `≈`-safe: `seam_fuses` (the empty-
/// body neck) assumes fusing and keeps a space; the operator-seam `debug_assert`
/// skips (a `HardPunct` boundary never fuses with an operator anyway, fusion arm [5]).
fn tok_of<'a>(ctx: &Ctx<'a>, node: Node) -> Option<Tok<'a>> {
    let k = ctx.kinds;
    let id = node.kind_id();
    let text = &ctx.src[node.byte_range()];
    let class = if id == k.neck {
        Class::HardPunct
    } else if id == k.colon {
        Class::Colon
    } else if id == k.dot {
        Class::HardPunct
    } else if id == k.identifier || id == k.variable {
        Class::Ident
    } else if id == k.number {
        Class::Number
    } else {
        return None;
    };
    Some(Tok { class, text })
}

/// Whether a trailing neck's last token would fuse with the tail's first — the fact
/// the empty-body seam self-heal consults (`assemble`'s `neck_tail_fuses`). `None` (an
/// unclassified seam token) is treated as fusing (keep a space; always `≈`-safe). No
/// neck or no tail → no abutment, hence no heal.
fn seam_fuses(ctx: &Ctx, neck: Option<Node>, tail_first: Option<Node>) -> bool {
    let (Some(n), Some(t)) = (neck, tail_first) else {
        return false;
    };
    match (tok_of(ctx, n), tok_of(ctx, t)) {
        (Some(l), Some(r)) => fuses(l, r),
        _ => true,
    }
}

/// The already-lowered Doc of `child` (spine invariant: a child is always in `built`
/// before its parent's `build` runs — the post-order guarantees it), wrapped with the
/// leading / trailing comments anchored to it ([`super::reinject::wrap`]).
/// The clean child (no attached comments) pays nothing.
fn child_doc(
    ctx: &Ctx,
    built: &FxHashMap<usize, NodeId>,
    child: Node,
    b: &mut DocBuilder,
) -> NodeId {
    let base = *built
        .get(&child.id())
        .expect("spine invariant: a child is lowered before its parent is built");
    super::reinject::wrap(ctx, base, child, b)
}

/// Detects a source blank line before an element of an exploded separated sequence,
/// off the same tree-sitter row signal the comment classifier uses. This is
/// the EXACT model `lower_source_file` uses at the statement level, applied per element:
/// the row anchor is threaded through EVERY child — elements, separators, AND re-injected
/// comments — and a blank seen before a non-element child is CARRIED to the element it
/// precedes.
///
/// Threading the anchor through every child is what makes detection both faithful and
/// idempotent. Measuring element-to-element instead would be (a) UNFAITHFUL — a comment
/// line between a separator and its element inflates the gap and reads as a phantom blank
/// — and (b) NON-IDEMPOTENT — when the formatter spreads a trailing comment or the
/// separator onto its own line, the element rows drift apart with no author blank. By
/// advancing the anchor over the separator / comment, the per-element gap is always to its
/// immediate predecessor (≤1 row absent an author blank, which survives as a `blank_line`
/// → re-parses to the same gap). The FIRST element has no predecessor, so a block-edge
/// blank just inside the opener stays 0.
struct ElementBlanks {
    prev_end_row: Option<usize>,
    carry: bool,
}

impl ElementBlanks {
    fn new() -> Self {
        Self {
            prev_end_row: None,
            carry: false,
        }
    }

    /// Whether a source blank line begins at `child` relative to the previous child, and
    /// advance the anchor to `child`'s end row.
    fn gap_blank(&mut self, child: Node) -> bool {
        let blank = self
            .prev_end_row
            .is_some_and(|p| child.start_position().row >= p + 2);
        self.prev_end_row = Some(child.end_position().row);
        blank
    }

    /// Note a NON-element child (a separator, or a comment re-injected via the element it
    /// rides with): advance the anchor and accumulate any blank before it, owed to the
    /// next element.
    fn note(&mut self, child: Node) {
        self.carry |= self.gap_blank(child);
    }

    /// Whether a source blank precedes this ELEMENT — a gap directly before it, or one
    /// carried from a preceding separator / comment; advances the anchor, consumes the carry.
    fn blank_before(&mut self, child: Node) -> bool {
        let blank = self.gap_blank(child) || self.carry;
        self.carry = false;
        blank
    }
}

/// [`child_doc`] for an element of an exploded separated sequence, prepended with a
/// blank-line marker when `blanks` reports a source blank before it. `blank_line`
/// is nothing when flat (so a non-exploding sequence is unaffected) and coalesces with
/// the sequence's own break when exploded — preserving exactly one author blank.
fn element_doc(
    ctx: &Ctx,
    built: &FxHashMap<usize, NodeId>,
    child: Node,
    blanks: &mut ElementBlanks,
    b: &mut DocBuilder,
) -> NodeId {
    let blank = blanks.blank_before(child);
    let doc = child_doc(ctx, built, child, b);
    if blank {
        let bl = b.blank_line();
        b.seq(&[bl, doc])
    } else {
        doc
    }
}

/// The style's block indent and neck hang as the assembler's `i32` nest deltas.
/// Checked, not `as`: a truncating cast would corrupt the layout (and trips
/// clippy-pedantic). The house constants are tiny, so the conversion never fails.
fn nest_deltas(style: &Style) -> (i32, i32) {
    let indent = i32::try_from(style.indent()).expect("indent fits in i32");
    let neck = i32::try_from(style.neck_width()).expect("neck_width fits in i32");
    (indent, neck)
}

/// Whether `node` degrades to its verbatim source span (the sanctioned `build → None`
/// totality backstop): a direct `ERROR` / `MISSING` span, or a pool totality edge —
/// a `lone_comma` (`(,)`) or an `empty_pool_item` (`f(a;;b)`). Degrading avoids
/// corruption: an `ERROR`-wrapped neck would collapse spacing (`a :- .` → `a:-.`); the
/// pool edge's exact author form is both prettier and provably `≈` verbatim. Comments no
/// longer degrade — leading / trailing are re-injected at their anchors (`child_doc`),
/// dangling via `push_dangling` (and `source_file`'s own in `lower_source_file`).
fn has_unformattable_child(ctx: &Ctx, node: Node) -> bool {
    let k = ctx.kinds;
    let mut degrade = false;
    crate::cst::walk_children(node, |_field, child| {
        if child.kind_id() == k.lone_comma
            || child.kind_id() == k.empty_pool_item
            || child.is_error()
            || child.is_missing()
        {
            degrade = true;
        }
    });
    degrade
}

#[cfg(test)]
mod tests {
    use crate::style::Style;

    /// Parse → attach → lower → render at `width` (the full emitter pipeline).
    /// The final-newline normalization is the `format` pass, so a single
    /// statement renders with no trailing newline here.
    fn fmt(src: &str, width: usize) -> String {
        let tree = crate::cst::parse(src);
        let at = crate::comments::attach(&tree, src);
        let doc = crate::emit::lower(&tree, src, &at, &Style::default());
        doc::render(&doc, src, width)
    }

    #[test]
    fn headed_rule_explodes_neck_trails_body_drops_indent_4() {
        // gallery [width 20]: the neck trails the head; the body drops to a
        // relative indent-4 block, one literal per line.
        let out = fmt("reachable(X, Y) :- edge(X, Z), reachable(Z, Y).\n", 20);
        assert_eq!(
            out,
            "reachable(X, Y) :-\n    edge(X, Z),\n    reachable(Z, Y)."
        );
    }

    #[test]
    fn headed_rule_stays_flat_when_it_fits() {
        let out = fmt("reachable(X, Y) :- edge(X, Z), reachable(Z, Y).\n", 100);
        assert_eq!(out, "reachable(X, Y) :- edge(X, Z), reachable(Z, Y).");
    }

    #[test]
    fn headed_rule_body_stays_flat_under_broken_neck_recursive_minimal() {
        // Recursive-minimal: the whole rule overflows so the neck breaks, but
        // the body fits at indent 4, so the body stays FLAT — not one-per-line.
        // This is the behavior the body's own inner group exists to produce.
        let out = fmt("reachable(X, Y) :- e(X, Y), e(Y, X).\n", 25);
        assert_eq!(out, "reachable(X, Y) :-\n    e(X, Y), e(Y, X).");
    }

    #[test]
    fn integrity_constraint_neck_leads_first_literal_rides_hang_3() {
        // gallery [width 20]: the neck leads, the first literal rides it, the
        // rest hang at the neck width (3).
        let out = fmt(":- assign(T, S1), assign(T, S2), S1 < S2.\n", 20);
        assert_eq!(out, ":- assign(T, S1),\n   assign(T, S2),\n   S1 < S2.");
    }

    #[test]
    fn integrity_constraint_stays_flat_when_it_fits() {
        let out = fmt(":- assign(T, S1), assign(T, S2), S1 < S2.\n", 100);
        assert_eq!(out, ":- assign(T, S1), assign(T, S2), S1 < S2.");
    }

    #[test]
    fn two_statements_are_joined_by_a_single_newline() {
        // The minimal source_file rule: statements on consecutive lines (the
        // blank-line rule and the final newline are handled separately).
        let out = fmt("a :- b.\nc :- d.\n", 100);
        assert_eq!(out, "a :- b.\nc :- d.");
    }

    #[test]
    fn interior_line_comment_is_reinjected_and_explodes_the_body() {
        // An interior line comment is re-injected at its anchor (trailing the
        // `,`, one-space gap) and forces the body to explode (a `%` comment runs to
        // end-of-line, so the next literal drops to its own line at the body indent).
        // This is rustfmt's behavior; it supersedes the old verbatim degrade.
        let out = fmt("a :- b, % note\n  c.\n", 100);
        assert_eq!(out, "a :-\n    b, % note\n    c.");
    }

    #[test]
    fn leading_comment_sits_on_its_own_line_above_the_statement() {
        assert_eq!(fmt("% header\na :- b.\n", 100), "% header\na :- b.");
    }

    #[test]
    fn a_run_of_leading_comments_shares_the_anchor_in_order() {
        assert_eq!(fmt("% one\n% two\na.\n", 100), "% one\n% two\na.");
    }

    #[test]
    fn trailing_comment_rides_its_statement_with_a_one_space_gap() {
        // rustfmt POLS: ONE space before a trailing comment; the comment's forced break
        // is dropped at end-of-file (the lazy renderer), so no trailing newline here.
        assert_eq!(fmt("a :- b. % note\n", 100), "a :- b. % note");
    }

    #[test]
    fn trailing_comment_whitespace_is_stripped() {
        assert_eq!(fmt("a. % note   \n", 100), "a. % note");
    }

    #[test]
    fn a_multi_line_block_comment_is_re_injected_byte_exact() {
        // A leading multi-line block comment is a verbatim span: its internal
        // newline and indentation pass through unchanged (no re-indent), then the
        // statement on its own line.
        assert_eq!(fmt("%* a\n  b *%\np.\n", 100), "%* a\n  b *%\np.");
    }

    #[test]
    fn single_line_block_between_tokens_forces_the_break_v0() {
        // v0: a single-line block comment between code tokens forces the break
        // (the in-place `Inline` role is reserved). It trails the `:-` neck,
        // and the body drops below it.
        let out = fmt("a :- %* note *% b.\n", 100);
        assert!(out.contains("%* note *%"), "comment preserved: {out:?}");
        assert!(out.contains('\n'), "the comment forces a break: {out:?}");
    }

    #[test]
    fn an_error_child_still_degrades_and_subsumes_its_comment() {
        // A node with an ERROR / MISSING child still degrades to its byte-exact verbatim
        // span (not collapsed). Re-injection does not run for a degraded node; an
        // interior comment is preserved inside the verbatim span, never double-emitted.
        let out = fmt("a :- . % c\n", 100);
        assert!(out.contains(":-"), "neck preserved: {out:?}");
        assert!(out.contains("% c"), "comment preserved: {out:?}");
    }

    #[test]
    fn dangling_comment_lands_before_the_closer_at_block_indent() {
        // A comment after the last element, before the bracket closer, is
        // re-injected on its own line at the block indent; the closer dedents below it.
        // The dangling comment forces the BRACKET to explode, but the argument list `a,
        // b` is its own inner group and stays flat since it fits (recursive-minimal).
        let out = fmt("p(a, b\n% after\n).\n", 100);
        assert_eq!(out, "p(\n    a, b\n    % after\n).");
    }

    #[test]
    fn comment_only_bracket_body_has_no_doubled_blank_line() {
        // The bracket's open-break and the lone dangling comment's leading hardline
        // coalesce (the lazy renderer), so there is no blank line after `(`.
        let out = fmt("p(\n% only\n).\n", 100);
        assert_eq!(out, "p(\n    % only\n).");
    }

    #[test]
    fn dangling_comment_before_a_statement_dot_keeps_the_dot() {
        // A comment on its own line between the body and the `.` dangles on the rule;
        // re-injecting it must not let the line comment swallow the `.` (mode A's
        // trailing hardline drops the dot to its own line).
        let out = fmt("a :- b\n% c\n.\n", 100);
        assert!(out.contains("% c"), "comment preserved: {out:?}");
        assert!(out.trim_end().ends_with('.'), "dot preserved: {out:?}");
    }

    #[test]
    fn comment_reinjection_is_depth_safe() {
        // Re-injection rides the same iterative work-list; a deep nest carrying a leading
        // comment must still return on a 512 KiB stack.
        let out = crate::test_support::run_on_tiny_stack(|| {
            let n = 2000;
            let src = format!("% c\np({}0{}).\n", "f(".repeat(n), ")".repeat(n));
            fmt(&src, 80)
        });
        assert!(
            out.starts_with("% c\np("),
            "deep nest with a comment must format: {:?}",
            &out[..out.len().min(12)]
        );
    }

    #[test]
    fn comment_layout_is_idempotent() {
        // format(format(x)) == format(x) for comment-bearing input: re-formatting
        // the output is a fixed point.
        let once = fmt("a :- b, % note\nc. % tail\n", 20);
        let twice_src = format!("{once}\n");
        assert_eq!(
            fmt(&twice_src, 20),
            once,
            "comment layout must be idempotent"
        );
    }

    #[test]
    fn error_bearing_statement_degrades_to_verbatim_not_collapse() {
        // A handled construct with a direct ERROR / MISSING child degrades to its
        // verbatim span rather than collapse spacing (`a :- .` must NOT become
        // `a:-.`). Production never formats error input (the CLI `has_error` skip),
        // but the emitter's totality backstop must still be byte-faithful here.
        let out = fmt("a :- .\n", 100);
        assert_eq!(out, "a :- .");
    }

    #[test]
    fn disjunctive_head_explodes_one_disjunct_per_line() {
        // gallery [width 22]: a disjunctive head is a separated sequence —
        // disjuncts at the base column with the `;` preserved as authored, then the
        // body drops to indent 4.
        let out = fmt("color(N, r); color(N, g); color(N, b) :- node(N).\n", 22);
        assert_eq!(
            out,
            "color(N, r);\ncolor(N, g);\ncolor(N, b) :-\n    node(N)."
        );
    }

    #[test]
    fn disjunctive_head_stays_flat_when_it_fits() {
        let out = fmt("a; b; c :- d.\n", 100);
        assert_eq!(out, "a; b; c :- d.");
    }

    #[test]
    fn disjunction_pipe_separator_is_spaced_both_sides() {
        // The `|` disjunction separator is spaced on BOTH sides (`a | b`),
        // an alternation bar — unlike the trailing `;` / `,` list separators
        // (`a; b`), which hug the left element.
        let out = fmt("a | b | c :- d.\n", 100);
        assert_eq!(out, "a | b | c :- d.");
    }

    #[test]
    fn disjunction_pipe_separator_trails_when_broken() {
        // General rule: the separator trails (rides the left disjunct), so a
        // broken `|`-disjunction keeps the bar at each line's end with its space.
        let out = fmt("aaa | bbb | ccc :- ddd.\n", 12);
        assert_eq!(out, "aaa |\nbbb |\nccc :-\n    ddd.");
    }

    // ----- bracketed aggregates (set_aggregate / choice) -----

    #[test]
    fn bounded_choice_explodes_body_drops_bounds_flank() {
        // gallery (bounded choice): the numeric bounds flank the brace spaced
        // (`1 {`, `} 1`); the brace block explodes one element per line with the
        // `;` trailing; the closer dedents and the upper bound rides it; the neck
        // trails `} 1`, and the head explosion drops the body to indent 4. Width 30
        // (not the gallery's illustrative 24, where the elements are 25-26 cols and
        // overflow): now that elements lower structurally, 30 keeps them flat so this
        // test isolates the choice-level shape; element recursive-minimal explosion is
        // covered by `bounded_choice_recursive_minimal_matches_spec_gallery_case_5`.
        let out = fmt(
            "1 { assign(T,S) : slot(S); waive(T) : optional(T) } 1 :- task(T).\n",
            30,
        );
        assert_eq!(
            out,
            "1 {\n    assign(T,S) : slot(S);\n    waive(T) : optional(T)\n} 1 :-\n    task(T)."
        );
    }

    #[test]
    fn bounded_choice_stays_flat_when_it_fits() {
        // Flat: bounds flank spaced, spaces inside the braces, `;` separators spaced.
        let out = fmt(
            "1 { assign(T,S) : slot(S); waive(T) : optional(T) } 1 :- task(T).\n",
            100,
        );
        assert_eq!(
            out,
            "1 { assign(T,S) : slot(S); waive(T) : optional(T) } 1 :- task(T)."
        );
    }

    #[test]
    fn choice_normalizes_brace_spacing_and_element_interior() {
        // The aggregate's own spacing is normalized (`1 {`, spaces inside, `} 1`), AND
        // — since `set_aggregate_element` is lowered structurally — the element interior
        // is normalized too: the conditional `:` becomes spaced (`assign(T,S) : slot(S)`),
        // while the function's depth-≥2 argument comma stays tight (`assign(T,S)`).
        // (Previously the element was a verbatim leaf preserving the authored `:slot`.)
        let out = fmt("1{assign(T,S):slot(S)}1 :- task(T).\n", 100);
        assert_eq!(out, "1 { assign(T,S) : slot(S) } 1 :- task(T).");
    }

    #[test]
    fn boundless_choice_brace_leads_its_own_line() {
        // Invariant: a boundless `{` with no bound/name is a heavy opener,
        // so it may lead its own line when the head explodes. Width 30 (not the
        // gallery's illustrative 24, where the elements overflow): keeps the elements
        // flat so this test isolates the boundless-opener shape (see the sibling
        // bounded-choice test's note).
        let out = fmt(
            "{ assign(T,S) : slot(S); waive(T) : optional(T) } :- task(T).\n",
            30,
        );
        assert_eq!(
            out,
            "{\n    assign(T,S) : slot(S);\n    waive(T) : optional(T)\n} :-\n    task(T)."
        );
    }

    #[test]
    fn boundless_choice_stays_flat_when_it_fits() {
        let out = fmt(
            "{ assign(T,S) : slot(S); waive(T) : optional(T) } :- task(T).\n",
            100,
        );
        assert_eq!(
            out,
            "{ assign(T,S) : slot(S); waive(T) : optional(T) } :- task(T)."
        );
    }

    #[test]
    fn boundless_choice_fact_explodes_dot_glued_to_closer() {
        // A choice fact is a headless rule (`{ … }.`): the dot rides the dedented
        // closer (`}.`, no space), the elements one per line.
        let out = fmt("{ a; b }.\n", 6);
        assert_eq!(out, "{\n    a;\n    b\n}.");
    }

    #[test]
    fn boundless_choice_fact_stays_flat_when_it_fits() {
        let out = fmt("{ a; b }.\n", 100);
        assert_eq!(out, "{ a; b }.");
    }

    #[test]
    fn empty_choice_has_no_interior_padding() {
        // An empty brace pair renders `{}` — never `{  }` (a double pad from the
        // open/close break collapsing onto each other) — and stays total.
        let out = fmt("{} :- a.\n", 100);
        assert_eq!(out, "{} :- a.");
    }

    // ----- optimization statements (minimize / maximize) -----

    #[test]
    fn minimize_explodes_keyword_abuts_brace_dot_glued() {
        // gallery (optimize): the `#minimize` keyword abuts its element brace
        // (`#minimize{`), the weight tuples explode one per line, and the statement dot
        // rides the dedented closer (`}.`). Width 26 (not the gallery's illustrative 24,
        // where `P@2, T : penalty(T,P)` is 25 cols and overflows): now that optimize
        // elements lower structurally, 26 keeps both flat so this test isolates the
        // keyword-abut / dot-glue; element explosion is covered by
        // `optimize_element_weight_prefix_is_spaced_and_condition_hangs`.
        let out = fmt(
            "#minimize { C@1, T : cost(T,C); P@2, T : penalty(T,P) }.\n",
            26,
        );
        assert_eq!(
            out,
            "#minimize{\n    C@1, T : cost(T,C);\n    P@2, T : penalty(T,P)\n}."
        );
    }

    #[test]
    fn minimize_stays_flat_when_it_fits() {
        let out = fmt(
            "#minimize { C@1, T : cost(T,C); P@2, T : penalty(T,P) }.\n",
            100,
        );
        assert_eq!(
            out,
            "#minimize{ C@1, T : cost(T,C); P@2, T : penalty(T,P) }."
        );
    }

    #[test]
    fn maximize_explodes_like_minimize() {
        let out = fmt("#maximize { C@1 : cost(C); P@2 : penalty(P) }.\n", 20);
        assert_eq!(
            out,
            "#maximize{\n    C@1 : cost(C);\n    P@2 : penalty(P)\n}."
        );
    }

    // ----- head aggregate (#count{…} >= n) -----

    #[test]
    fn head_aggregate_function_abuts_brace_upper_rides_closer() {
        // The aggregate function abuts its brace (`#count{`); the relational upper
        // bound rides the dedented closer spaced (`} >= 1`); the neck trails it and
        // the head explosion drops the body.
        let out = fmt("#count { X : p(X) } >= 1 :- q.\n", 12);
        assert_eq!(out, "#count{\n    X : p(X)\n} >= 1 :-\n    q.");
    }

    #[test]
    fn head_aggregate_stays_flat_when_it_fits() {
        let out = fmt("#count { X : p(X) } >= 1 :- q.\n", 100);
        assert_eq!(out, "#count{ X : p(X) } >= 1 :- q.");
    }

    #[test]
    fn head_aggregate_lower_bound_is_spaced_from_function() {
        // A relational lower bound leads the function, spaced (`1 <= #count{`).
        let out = fmt("1 <= #count { X : p(X) } :- q.\n", 14);
        assert_eq!(out, "1 <= #count{\n    X : p(X)\n} :-\n    q.");
    }

    #[test]
    fn aggregate_guard_relation_is_normalized_spaced() {
        // An aggregate bound's `relation` is spaced both sides like a comparison
        // and NORMALIZED — independent of the input's spacing (the formatter does not
        // echo source whitespace). Regression guard: `lower` / `upper` had no lowering
        // and fell through to the verbatim default, so `{a}=1` kept its tight `=1`.
        assert_eq!(fmt("{a}=1.\n", 100), "{ a } = 1.");
        assert_eq!(fmt("{ a }  =  1 .\n", 100), "{ a } = 1.");
        assert_eq!(fmt(":- #count{ x } >=2.\n", 100), ":- #count{ x } >= 2.");
        // both a lower and an upper bound on the one aggregate:
        assert_eq!(fmt("1<{a}<3.\n", 100), "1 < { a } < 3.");
    }

    // ----- theory atom (&name{…} op term) -----

    #[test]
    fn theory_atom_name_abuts_brace_upper_rides_closer() {
        // `&name` abuts its element brace (`&diff{`); the theory upper bound rides
        // the dedented closer spaced (`} <= 3`); the neck trails and the body drops.
        let out = fmt("&diff { x - y } <= 3 :- a.\n", 10);
        assert_eq!(out, "&diff{\n    x - y\n} <= 3 :-\n    a.");
    }

    #[test]
    fn theory_atom_stays_flat_when_it_fits() {
        let out = fmt("&diff { x - y } <= 3 :- a.\n", 100);
        assert_eq!(out, "&diff{ x - y } <= 3 :- a.");
    }

    #[test]
    fn theory_atom_arguments_abut_the_name() {
        // A theory atom's `(args)` abut the name (`&p(1){`), like a function's.
        let out = fmt("&p(1) { q } :- a.\n", 8);
        assert_eq!(out, "&p(1){\n    q\n} :-\n    a.");
    }

    // ----- body literals, signs, comparisons -----

    #[test]
    fn comparison_relations_are_spaced() {
        // A body comparison's `relation` is spaced both sides; the terms are
        // verbatim, so `X<Y` normalizes to `X < Y`.
        let out = fmt("a :- X<Y.\n", 100);
        assert_eq!(out, "a :- X < Y.");
    }

    #[test]
    fn chained_comparison_relations_are_spaced() {
        let out = fmt("a :- X<Y<Z.\n", 100);
        assert_eq!(out, "a :- X < Y < Z.");
    }

    #[test]
    fn default_negation_is_spaced_from_its_atom() {
        // `not` is always spaced from the following atom (lest `not p` fuse to
        // `notp`); the extra authored space is normalized to one.
        let out = fmt("a :- not  b.\n", 100);
        assert_eq!(out, "a :- not b.");
    }

    #[test]
    fn double_default_negation_is_spaced() {
        let out = fmt("a :- not not b.\n", 100);
        assert_eq!(out, "a :- not not b.");
    }

    #[test]
    fn sign_less_body_literal_is_unchanged() {
        // Regression: descending into body_literal must not alter a sign-less
        // verbatim atom (the body_literal span equals its atom span).
        let out = fmt("reachable(X, Y) :- edge(X, Z), reachable(Z, Y).\n", 100);
        assert_eq!(out, "reachable(X, Y) :- edge(X, Z), reachable(Z, Y).");
    }

    #[test]
    fn body_aggregate_in_constraint_composes_hang_increments() {
        // gallery (constraint nesting a body aggregate): the headless neck leads
        // with `task(T)` riding it, the body hangs at the neck width (3), then the
        // aggregate's elements hang at 3+4=7; the closer dedents to the aggregate's base
        // column (3) with `} >= 2` glued. Increments compose. Width 24 (not the gallery's
        // illustrative 16, where `S : assign(T,S)` is 22 cols and overflows): now that the
        // element lowers structurally, 24 keeps it flat so this test isolates the
        // hang-composition; the element's own recursive-minimal explosion is covered by
        // `body_aggregate_element_condition_explodes_under_recursive_minimal`.
        let out = fmt(":- task(T), #count { S : assign(T,S) } >= 2.\n", 24);
        assert_eq!(
            out,
            ":- task(T),\n   #count{\n       S : assign(T,S)\n   } >= 2."
        );
    }

    #[test]
    fn body_aggregate_stays_flat_when_it_fits() {
        let out = fmt(":- task(T), #count { S : assign(T,S) } >= 2.\n", 100);
        assert_eq!(out, ":- task(T), #count{ S : assign(T,S) } >= 2.");
    }

    #[test]
    fn empty_constraint_body_is_total() {
        // `:- .` parses as a constraint with NO body node; the leading neck renders
        // it without crashing (an empty separated sequence is nil, the hang nests
        // nothing) and round-trips byte-faithfully.
        assert_eq!(fmt(":- .\n", 100), ":- .");
        // `:-.` (no space) normalizes to `:- .` — the neck's invariant trailing
        // space is inserted; ≈-safe (same `integrity_constraint(:- , .)` token run).
        assert_eq!(fmt(":-.\n", 100), ":- .");
    }

    #[test]
    fn aggregate_with_an_interior_comment_degrades_to_verbatim() {
        // A comment inside the element list can't be laid out faithfully until
        // comment re-injection, so the element list degrades to its
        // verbatim source span (byte-exact, ≈-safe: the `;`, both atoms, AND the
        // comment all survive) rather than risk the comment swallowing a token.
        let out = fmt("{ a; % note\nb } :- c.\n", 100);
        assert!(out.contains("% note"), "the comment must survive: {out:?}");
        assert!(out.contains("a;"), "the elements must survive: {out:?}");
        assert!(out.contains('b'), "the elements must survive: {out:?}");
    }

    // ----- conditional literals (`literal : condition`) -----

    #[test]
    fn conditional_literal_condition_hangs_one_further_level() {
        // The conditional `:` is a connective opener that trails the literal,
        // so when the conditional literal breaks the condition hangs one further
        // level. Here the rule breaks (body drops to indent 4) and the conditional
        // literal itself breaks (the condition `c, d` drops to indent 8 — 4 + 4 —
        // and, fitting there, stays flat). The bare literal verbatim would overflow
        // as one blob; descending into the `:` is what lets it break.
        let out = fmt("a :- bbb : c, d.\n", 14);
        assert_eq!(out, "a :-\n    bbb :\n        c, d.");
    }

    #[test]
    fn conditional_literal_empty_condition_is_total() {
        // `p :` — a conditional literal with NO condition node — is a valid,
        // error-free parse (it reaches the emitter, not the verbatim degrade). The
        // trailing neck over an empty body must hang NOTHING: `p :`, the `:` spaced
        // before with no trailing space, the rule's `.` then abutting (`p :.`).
        // (Mirrors the leading neck's bodyless `:- .`; without it the unconditional
        // drop-line leaves a spurious space after the `:`.)
        assert_eq!(fmt("a :- p :.\n", 100), "a :- p :.");
        // The wider literal form, same shape.
        assert_eq!(fmt("a :- p(X) :.\n", 100), "a :- p(X) :.");
    }

    #[test]
    fn set_aggregate_element_condition_explodes_under_recursive_minimal() {
        // A choice/aggregate element (`set_aggregate_element`) is structurally
        // a conditional literal (`literal : condition`), so when the element overflows
        // its line the condition must hang one further level — exactly as a
        // body-level conditional does. Before the fix the element degraded to an
        // unbreakable verbatim leaf and overflowed the width.
        let out = fmt("{ a(X) : c1(X), c2(X) }.\n", 16);
        assert_eq!(out, "{\n    a(X) :\n        c1(X),\n        c2(X)\n}.");
    }

    #[test]
    fn bounded_choice_recursive_minimal_matches_spec_gallery_case_5() {
        // The gallery's recursive-minimal example, width 28:
        // the first element's condition explodes (it overflows); the short second
        // element stays flat; the exploded head still drops the body.
        let out = fmt(
            "1 { assign(T,S) : slot(S), available(S); waive(T) : optional(T) } 1 :- task(T).\n",
            28,
        );
        assert_eq!(
            out,
            "1 {\n    assign(T,S) :\n        slot(S),\n        available(S);\n    waive(T) : optional(T)\n} 1 :-\n    task(T)."
        );
    }

    #[test]
    fn body_aggregate_element_condition_explodes_under_recursive_minimal() {
        // `body_aggregate_element` is `terms : condition` — the same conditional-literal
        // shape; its condition must hang when the element overflows (same root-cause fix).
        let out = fmt(":- #count{ X : c1(X), c2(X) } >= 1.\n", 18);
        assert_eq!(
            out,
            ":- #count{\n       X :\n           c1(X),\n           c2(X)\n   } >= 1."
        );
    }

    #[test]
    fn optimize_element_weight_prefix_is_spaced_and_condition_hangs() {
        // `optimize_element` is `weight , terms : condition` — the `weight, terms` prefix
        // is a spaced comma-sequence (the trailing-neck head), then `: condition` is the
        // conditional connective whose condition hangs when the element overflows.
        let out = fmt("#minimize{ C@1, T : cost(T,C) }.\n", 20);
        assert_eq!(out, "#minimize{\n    C@1, T :\n        cost(T,C)\n}.");
    }

    #[test]
    fn optimize_element_normalizes_interior_spacing() {
        // Authored tight (`C@1,T:cost(T,C)`); the comma is spaced (depth-1) and the
        // conditional `:` is spaced both sides — no longer a verbatim passthrough.
        let out = fmt("#minimize{C@1,T:cost(T,C)}.\n", 100);
        assert_eq!(out, "#minimize{ C@1, T : cost(T,C) }.");
    }

    #[test]
    fn head_aggregate_element_double_colon_normalizes_and_first_colon_hangs() {
        // `head_aggregate_element` is `terms : literal [: condition]`. The FIRST `:` is
        // the connective (the literal hangs under the terms); a SECOND `:` (the guard
        // condition) is a spaced trailing separator riding with the literal in the body.
        // Flat: both colons spaced.
        assert_eq!(
            fmt("1 #count{ X : p(X) : q(X) }.\n", 100),
            "1 #count{ X : p(X) : q(X) }."
        );
        // Broken: the literal hangs one level under the terms (width 28 — the smallest
        // where the hung `plongername(Xlong)` fits at indent 8, isolating the first-colon
        // hang from the literal's own arg explosion).
        let out = fmt("1 #count{ Xlong : plongername(Xlong) }.\n", 28);
        assert_eq!(
            out,
            "1 #count{\n    Xlong :\n        plongername(Xlong)\n}."
        );
    }

    #[test]
    fn conditional_literal_stays_flat_when_it_fits() {
        // Flat: the `:` spaced both sides, the condition a spaced `,`-list.
        let out = fmt("a :- bbb : c, d.\n", 100);
        assert_eq!(out, "a :- bbb : c, d.");
    }

    #[test]
    fn conditional_literal_condition_explodes_one_per_line() {
        // Narrower than the hang-one-further-level case: now the condition itself
        // overflows at indent 8, so it explodes one literal per line with the `,`
        // trailing — the separated-sequence rule, two levels under the rule body.
        let out = fmt("a :- bbb : c, d.\n", 12);
        assert_eq!(out, "a :-\n    bbb :\n        c,\n        d.");
    }

    #[test]
    fn conditional_literal_condition_comma_is_spaced() {
        // The condition is a depth-0 separated sequence, so its `,` is spaced
        // (`c,d` normalizes to `c, d`) exactly like a body conjunction.
        let out = fmt("a :- b : c,d.\n", 100);
        assert_eq!(out, "a :- b : c, d.");
    }

    #[test]
    fn conditional_literal_as_head_composes_via_disjunction() {
        // A head conditional literal (`a : b.`) parses as a single-element
        // `disjunction` wrapping the conditional literal, so it composes through
        // the separated-sequence path with no special case.
        assert_eq!(fmt("a : b.\n", 100), "a : b.");
        // Broken: the condition hangs one further level under the conditional `:`.
        assert_eq!(fmt("aaa : bbb.\n", 8), "aaa :\n    bbb.");
    }

    #[test]
    fn conditional_colon_never_abuts_a_following_classical_negation() {
        // Witness: the conditional `:` must never abut a following `-`
        // (`:-` would fuse into a neck). A condition opening with a classically
        // negated literal keeps the space: `b : -c`, never `b :-c`.
        let out = fmt("a :- b : -c.\n", 100);
        assert_eq!(out, "a :- b : -c.");
    }

    // ----- applicative atoms / functions, argument lists & pools -----

    #[test]
    fn predicate_argument_comma_is_spaced_at_depth_1() {
        // A single-bracket argument comma is SPACED (depth 1), so a tight
        // source `edge(X,Y)` normalizes to `edge(X, Y)` — in head AND body.
        assert_eq!(
            fmt("edge(X,Y) :- node(X),node(Y).\n", 100),
            "edge(X, Y) :- node(X), node(Y)."
        );
    }

    #[test]
    fn nested_argument_comma_tightens_at_depth_2() {
        // A comma two brackets deep is TIGHT. The outer arg comma stays
        // spaced (depth 1); the inner comma tightens (depth 2) — a SPACED-source
        // inner `g(Y, Z)` must close up to `g(Y,Z)`.
        assert_eq!(fmt("p(X, g(Y, Z)).\n", 100), "p(X, g(Y,Z)).");
    }

    #[test]
    fn classical_negation_carries_through_and_args_normalize() {
        // Classical `-` abuts its name (tight); the args still normalize at
        // their depth — `-p(X,Y)` tightens nothing at the neg but spaces the comma.
        assert_eq!(fmt("-p(X,Y) :- q(X).\n", 100), "-p(X, Y) :- q(X).");
    }

    #[test]
    fn argument_pool_semicolon_is_spaced_at_depth_1() {
        // A 2-segment argument pool: the `;` is depth-1, so SPACED. A
        // tight source `f(a;b)` normalizes to `f(a; b)`.
        assert_eq!(fmt("p :- f(a;b).\n", 100), "p :- f(a; b).");
    }

    #[test]
    fn nested_argument_pool_tightens_at_depth_2() {
        // The pool `;` shares the comma threshold: two brackets deep it tightens.
        assert_eq!(fmt("p :- g(f(a; b)).\n", 100), "p :- g(f(a;b)).");
    }

    #[test]
    fn zero_ary_atom_is_unchanged() {
        assert_eq!(fmt("p :- q.\n", 100), "p :- q.");
    }

    #[test]
    fn external_function_at_abuts_name_and_args_normalize() {
        // `@f(...)` is a term: the `@` abuts the name, the args normalize at depth 1.
        assert_eq!(fmt("p(X) :- X = @f(1,2).\n", 100), "p(X) :- X = @f(1, 2).");
    }

    #[test]
    fn long_argument_list_explodes_one_per_line() {
        // Separated-sequence: the arg list fit-or-explodes, comma trailing,
        // recursive-minimal, at indent 4 inside the parens.
        let out = fmt("p(alpha, beta, gamma) :- q.\n", 12);
        assert_eq!(out, "p(\n    alpha,\n    beta,\n    gamma\n) :-\n    q.");
    }

    // ----- tuples & the semantic comma -----

    #[test]
    fn multi_tuple_comma_is_spaced_at_depth_1() {
        // A `(a, b)` tuple is a bracketed term; its depth-1 comma is spaced (a tight
        // source `(a,b)` normalizes to `(a, b)`).
        assert_eq!(fmt("p(X) :- X = (a,b).\n", 100), "p(X) :- X = (a, b).");
    }

    #[test]
    fn zero_tuple_is_empty_parens() {
        // `()` is the 0-tuple — empty parens, never `( )`.
        assert_eq!(fmt("p :- q = ().\n", 100), "p :- q = ().");
    }

    #[test]
    fn one_tuple_comma_is_preserved_hugged() {
        // `(a,)` is a 1-tuple, arity-distinct from `(a)`. The trailing
        // comma is SEMANTIC — preserved, hugged (never `(a, )`, never `(a)`).
        assert_eq!(fmt("p(X) :- X = (a,).\n", 100), "p(X) :- X = (a,).");
    }

    #[test]
    fn nested_tuple_comma_tightens_at_depth_2() {
        // A tuple is a bracket: its interior is depth+1. A tuple inside a function
        // arg is two brackets deep, so its comma tightens — `f((a, b))` → `f((a,b))`.
        assert_eq!(fmt("p :- f((a, b)).\n", 100), "p :- f((a,b)).");
    }

    #[test]
    fn tuple_pool_semicolon_is_spaced_at_depth_1() {
        // A `(a; b)` tuple pool: the `;` is depth-1, so spaced.
        assert_eq!(fmt("p(X) :- X = (a;b).\n", 100), "p(X) :- X = (a; b).");
    }

    #[test]
    fn lone_comma_tuple_is_preserved_verbatim() {
        // `(,)` — a bare comma — is a totality edge case, preserved as authored.
        assert_eq!(fmt("p(X) :- X = (,).\n", 100), "p(X) :- X = (,).");
    }

    #[test]
    fn empty_pool_segment_is_preserved_verbatim() {
        // An empty pool segment (`f(a;;b)`) is a totality edge case — preserve the
        // author's exact form rather than render the empty segment with stray spaces.
        assert_eq!(fmt("p :- f(a;;b).\n", 100), "p :- f(a;;b).");
    }

    // ----- term operators (binary, unary, interval) -----

    #[test]
    fn term_operator_is_spaced_at_top_body_level() {
        // A term operator breathes at the top body level — a tight `Y*2`
        // normalizes to `Y * 2`.
        assert_eq!(fmt("p(X) :- X = Y*2.\n", 100), "p(X) :- X = Y * 2.");
    }

    #[test]
    fn term_operator_tightens_inside_a_bracket() {
        // Tight inside any bracket — a spaced `Y + Z` inside an arg list closes
        // up to `Y+Z`.
        assert_eq!(fmt("p :- q(X, Y + Z).\n", 100), "p :- q(X, Y+Z).");
    }

    #[test]
    fn interval_is_always_tight() {
        // `..` is ALWAYS tight, even at the top body level — a spaced
        // `1 .. 3` closes up to `1..3`.
        assert_eq!(fmt("p(X) :- X = 1 .. 3.\n", 100), "p(X) :- X = 1..3.");
    }

    #[test]
    fn unary_minus_abuts_its_operand() {
        // Unary `-` is tight — `- Y` normalizes to `-Y`.
        assert_eq!(fmt("p(X) :- X = - Y.\n", 100), "p(X) :- X = -Y.");
    }

    #[test]
    fn torture_term_round_trips_tight_inside_brackets() {
        // The oracle: every fixed term-op seam abuts safely inside a
        // bracket (depth 1, all tight). `+-`→ADD·SUB, `**-`→POW·SUB, `2**`→`2`·POW,
        // `0..1`→NUM·DOTS·NUM (clingo has no float literals). Round-trips exactly.
        assert_eq!(
            fmt("p :- q(1+-2*2**-3**0..1-2).\n", 100),
            "p :- q(1+-2*2**-3**0..1-2)."
        );
    }

    #[test]
    fn top_level_operator_chain_spaces_uniformly() {
        // At the top body level operators breathe; a flat chain spaces every op
        // (precedence-agnostic, FLAT v0). `X = 1+2*3` → `X = 1 + 2 * 3`.
        assert_eq!(fmt("p(X) :- X = 1+2*3.\n", 100), "p(X) :- X = 1 + 2 * 3.");
    }

    // ----- abs |t| / |t; ...| -----

    #[test]
    fn abs_singleton_is_tight() {
        // `|Y|` hugs the bars (Pad::Tight).
        assert_eq!(fmt("p(X) :- X = |Y|.\n", 100), "p(X) :- X = |Y|.");
    }

    #[test]
    fn abs_pool_semicolon_is_spaced_at_depth_1() {
        // An abs pool `|a; b|`: the `;` is depth-1 (abs is a bracket, +depth), spaced.
        assert_eq!(fmt("p(X) :- X = |a;b|.\n", 100), "p(X) :- X = |a; b|.");
    }

    #[test]
    fn abs_interior_term_operator_is_tight() {
        // `|X-F|` is tight — abs counts as a bracket (+depth), so the interior
        // term op is at depth 1 → tight. A spaced `|Y - Z|` closes up to `|Y-Z|`.
        assert_eq!(fmt("p(X) :- X = |Y - Z|.\n", 100), "p(X) :- X = |Y-Z|.");
    }

    #[test]
    fn abs_nested_pool_tightens_at_depth_2() {
        // `|f(a; b)|`: the inner pool `;` is two brackets deep (abs + parens) → tight.
        assert_eq!(
            fmt("p(X) :- X = |f(a; b)|.\n", 100),
            "p(X) :- X = |f(a;b)|."
        );
    }

    // ----- golden forms (the house style `verify` cannot check) -----

    #[test]
    fn family4_golden_flat_forms() {
        // Worked house-style forms at width 100 (flat). Each `want` is pinned from an
        // actual run (the depth rules: `f(b,c)` at depth 2 is TIGHT inside `q(…)`).
        for (src, want) in [
            ("p(X,Y):-q(X,Y).\n", "p(X, Y) :- q(X, Y)."),
            ("p:-q(a,f(b,c)).\n", "p :- q(a, f(b,c))."),
            ("p(X):-X=1..n.\n", "p(X) :- X = 1..n."),
            ("p(X):-X= -Y* (Z+1).\n", "p(X) :- X = -Y * (Z+1)."),
            ("p(X):-X=|a;b|.\n", "p(X) :- X = |a; b|."),
        ] {
            assert_eq!(fmt(src, 100), want, "golden: {src:?}");
        }
    }

    // ----- theory elements & term lists -----

    #[test]
    fn theory_element_multi_term_comma_is_spaced() {
        // A theory element's comma-list normalizes at depth 1 (inside the atom brace):
        // a tight source `x,y` becomes `x, y`.
        assert_eq!(fmt("&a { x,y } :- b.\n", 100), "&a{ x, y } :- b.");
    }

    #[test]
    fn theory_element_condition_lays_out_like_a_conditional() {
        // `terms : condition` is the conditional-literal shape; the `:` is spaced both
        // sides and the condition is a `,`-list.
        assert_eq!(fmt("&a { x : p,q } :- b.\n", 100), "&a{ x : p, q } :- b.");
    }

    #[test]
    fn theory_kind_ids_resolve() {
        let k = crate::cst::KindIds::new();
        for id in [
            k.theory_function,
            k.theory_tuple,
            k.theory_list,
            k.theory_set,
            k.theory_unparsed_term,
            k.theory_terms,
            k.theory_operators,
            k.theory_element,
            k.theory_atom_upper,
            k.theory_atom_definition,
            k.comma,
            k.lbracket,
            k.rbracket,
        ] {
            assert_ne!(id, 0, "theory kind id must resolve");
        }
    }

    // ----- theory operators (always spaced, never tightened) -----

    #[test]
    fn theory_binary_operator_is_spaced() {
        // A theory operator is rendered with surrounding spaces — a tight source
        // `x-y` normalizes to `x - y` (≈-safe: `x`,`-`,`y` either way).
        assert_eq!(fmt("&a { x-y } :- b.\n", 100), "&a{ x - y } :- b.");
    }

    #[test]
    fn theory_unary_operator_gets_a_space() {
        // The accepted consequence of the always-spaced rule: a
        // unary theory operator is spaced from its operand — `-x` becomes `- x`.
        assert_eq!(fmt("&a { -x } :- b.\n", 100), "&a{ - x } :- b.");
    }

    #[test]
    fn theory_adjacent_operators_stay_spaced() {
        // The greedy-munch carve-out: two adjacent theory operators NEVER abut
        // (`+ +` ≠ `++`). An already-spaced `+ +` is preserved as two tokens.
        assert_eq!(fmt("&a { x + + y } :- b.\n", 100), "&a{ x + + y } :- b.");
    }

    #[test]
    fn theory_operator_chain_spaces_uniformly() {
        assert_eq!(fmt("&a { x*y+z } :- b.\n", 100), "&a{ x * y + z } :- b.");
    }

    // ----- bracketed theory terms (function / tuple / list / set) -----

    #[test]
    fn theory_function_args_normalize() {
        // A theory function's `(args)` abut the name; the comma-list tightens (depth 2
        // inside the atom brace + parens) — a spaced source `f(x, y)` closes to `f(x,y)`.
        assert_eq!(fmt("&a { f(x, y) } :- b.\n", 100), "&a{ f(x,y) } :- b.");
    }

    #[test]
    fn theory_tuple_and_one_tuple_comma() {
        assert_eq!(fmt("&a { (x, y) } :- b.\n", 100), "&a{ (x,y) } :- b.");
        // The 1-tuple trailing comma is semantic — preserved, hugged.
        assert_eq!(fmt("&a { (x,) } :- b.\n", 100), "&a{ (x,) } :- b.");
    }

    #[test]
    fn theory_list_and_set_brackets_are_tight() {
        assert_eq!(fmt("&a { [x, y] } :- b.\n", 100), "&a{ [x,y] } :- b.");
        assert_eq!(fmt("&a { {x, y} } :- b.\n", 100), "&a{ {x,y} } :- b.");
    }

    #[test]
    fn theory_lone_comma_tuple_degrades_to_verbatim() {
        // `(,)` is a totality edge case — preserved as authored.
        assert_eq!(fmt("&a { (,) } :- b.\n", 100), "&a{ (,) } :- b.");
    }

    // ----- theory atom upper bound (op term) -----

    #[test]
    fn theory_atom_upper_normalizes_operator_spacing() {
        // The upper `op term` — a tight source `<=3` becomes `<= 3` (always-spaced
        // theory operator). The leading space before `<=` comes from the `}` closer tail.
        assert_eq!(fmt("&a { x } <=3 :- b.\n", 100), "&a{ x } <= 3 :- b.");
    }

    #[test]
    fn theory_atom_upper_formats_its_term() {
        // The upper's term descends like any theory term — `x+y` → `x + y`.
        assert_eq!(fmt("&a { p } = x+y :- b.\n", 100), "&a{ p } = x + y :- b.");
    }

    #[test]
    fn theory_atom_upper_existing_family2_form_unchanged() {
        // Regression: the already-spaced form is byte-identical.
        assert_eq!(
            fmt("&diff { x - y } <= 3 :- a.\n", 100),
            "&diff{ x - y } <= 3 :- a."
        );
    }

    // ----- #theory definitions -----

    #[test]
    fn theory_operator_definition_unary_and_binary() {
        // `op : priority, arity[, associativity]` — the `,` field separators normalize
        // to `, ` (always spaced). The operator MUST be authored spaced from the `:`
        // (a tight `-:` greedy-munches into one theory operator — invalid input), so the
        // `op :` seam is already spaced; the formatter normalizes the comma seams.
        assert_eq!(
            fmt("#theory t { d { - : 0,unary } }.\n", 100),
            "#theory t { d { - : 0, unary } }."
        );
        assert_eq!(
            fmt("#theory t { d { + : 1,binary,left } }.\n", 100),
            "#theory t { d { + : 1, binary, left } }."
        );
    }

    #[test]
    fn theory_block_explodes_one_definition_per_line() {
        // The #theory block is a Pad::Spaced bracket; definitions explode one per line
        // (the `;` trailing) at indent 4 with `}.` dedented.
        let out = fmt(
            "#theory t { d1 { - : 0, unary }; d2 { + : 0, unary } }.\n",
            30,
        );
        assert_eq!(
            out,
            "#theory t {\n    d1 { - : 0, unary };\n    d2 { + : 0, unary }\n}."
        );
    }

    #[test]
    fn theory_directive_stays_flat_when_it_fits() {
        assert_eq!(
            fmt("#theory t { d { - : 0, unary } }.\n", 100),
            "#theory t { d { - : 0, unary } }."
        );
    }

    #[test]
    fn theory_atom_definition_minimal() {
        // `&name/arity : term_name, atom_type` — `&n/a` tight, ` : ` spaced, the `,`
        // field separator normalizes to `, `.
        assert_eq!(
            fmt("#theory t { &a/0 : trm,any }.\n", 100),
            "#theory t { &a/0 : trm, any }."
        );
    }

    #[test]
    fn theory_atom_definition_with_guard_set() {
        // The guard `{ ops }` is a Pad::Tight set with always-spaced operator commas
        // (`{<=, >=}`); the surrounding field `,`s normalize to `, `.
        assert_eq!(
            fmt("#theory t { &a/0 : trm,{<=,>=},g,body }.\n", 100),
            "#theory t { &a/0 : trm, {<=, >=}, g, body }."
        );
    }

    #[test]
    fn theory_worked_golden_definition() {
        // Worked #theory case — a term definition + an atom definition, flat.
        // (Operators are authored spaced from the `:` — a tight `-:` would munch.)
        let src = "#theory diff { diff_term { - : 0,binary,left }; &diff/0 : diff_term,any }.\n";
        assert_eq!(
            fmt(src, 100),
            "#theory diff { diff_term { - : 0, binary, left }; &diff/0 : diff_term, any }."
        );
    }

    #[test]
    fn theory_bare_colon_element_has_no_dangling_space() {
        // A degenerate theory element that is a bare `:` (a `_condition` with no terms
        // and no condition) hangs nothing: `&a{ : }`, NOT `&a{ :  }`. The theory family is the
        // first construct to reach `leading_neck` with empty body AND empty tail.
        assert_eq!(fmt("&a { : } :- b.\n", 100), "&a{ : } :- b.");
    }

    #[test]
    fn theory_element_condition_with_leading_sign_never_fuses_neck() {
        // Witness: the element `:` must never abut a following classical `-`
        // (`:-` would fuse into a neck). `x : -p` stays spaced, never `x :-p`.
        assert_eq!(fmt("&a { x : -p } :- b.\n", 100), "&a{ x : -p } :- b.");
    }

    #[test]
    fn family5_golden_flat_forms() {
        // Worked house-style theory forms at width 100 (flat), each pinned from an actual
        // run. The depth rules: a function/list/set interior at depth 2 (atom brace +
        // bracket) is TIGHT; theory operators are always spaced (incl. the unary `- x`).
        for (src, want) in [
            ("&a{x-y}:-b.\n", "&a{ x - y } :- b."),
            ("&a{-x}:-b.\n", "&a{ - x } :- b."), // always-spaced unary
            ("&a{f(x,y)}<=3:-b.\n", "&a{ f(x,y) } <= 3 :- b."),
            ("&a{(x,)}:-b.\n", "&a{ (x,) } :- b."),
            ("&a{[x,y];{z}}:-b.\n", "&a{ [x,y]; {z} } :- b."),
        ] {
            assert_eq!(fmt(src, 100), want, "golden: {src:?}");
        }
    }

    // ----- signature directives -----

    #[test]
    fn show_bare_glues_its_dot() {
        assert_eq!(fmt("#show .\n", 100), "#show.");
    }

    #[test]
    fn defined_signature_is_tight_keyword_spaced() {
        // `#defined` is spaced from the signature; the signature's `/` is tight.
        assert_eq!(fmt("#defined p / 1 .\n", 100), "#defined p/1.");
    }

    #[test]
    fn show_signature_with_boolean_tail() {
        assert_eq!(fmt("#show p/1. [true]\n", 100), "#show p/1. [true]");
    }

    #[test]
    fn project_signature_classical_negation_in_signature() {
        // `atom_identifier`'s optional `-` abuts the name (`-p/1`).
        assert_eq!(fmt("#project -p/2.\n", 100), "#project -p/2.");
    }

    #[test]
    fn directive_kind_ids_resolve() {
        let k = crate::cst::KindIds::new();
        for id in [k.signature, k.weight, k.parameters, k.edge_pair, k.equals] {
            assert_ne!(id, 0);
        }
    }

    // ----- const & include -----

    #[test]
    fn const_binding_is_spaced_both_sides() {
        assert_eq!(fmt("#const n=3.\n", 100), "#const n = 3.");
    }

    #[test]
    fn const_with_default_tail() {
        assert_eq!(
            fmt("#const n = 3. [default]\n", 100),
            "#const n = 3. [default]"
        );
    }

    #[test]
    fn const_with_override_tail_and_arith_value() {
        // The value is a `_const_term`; `1+2` is its own layout, here
        // tight at depth 0? No — a bare top-level const arith is spaced (`1 + 2`);
        // pin from the run.
        assert_eq!(
            fmt("#const n = 1+2. [override]\n", 100),
            "#const n = 1 + 2. [override]"
        );
    }

    #[test]
    fn include_string_is_keyword_spaced() {
        assert_eq!(fmt("#include \"lib.lp\".\n", 100), "#include \"lib.lp\".");
    }

    #[test]
    fn include_angled_identifier_is_tight() {
        assert_eq!(fmt("#include <incmode>.\n", 100), "#include <incmode>.");
    }

    // ----- colon-body directives -----

    #[test]
    fn show_term_with_condition_flat() {
        assert_eq!(fmt("#show f(X):p(X).\n", 100), "#show f(X) : p(X).");
    }

    #[test]
    fn show_term_no_condition() {
        // `#show t.` with no `: body` — the colon-body is just the `.`.
        assert_eq!(fmt("#show f(a).\n", 100), "#show f(a).");
    }

    #[test]
    fn external_no_body_with_type_tail() {
        assert_eq!(
            fmt("#external p(X). [true]\n", 100),
            "#external p(X). [true]"
        );
    }

    #[test]
    fn external_body_and_type_tail() {
        assert_eq!(
            fmt("#external p(X) : q(X). [false]\n", 100),
            "#external p(X) : q(X). [false]"
        );
    }

    #[test]
    fn project_atom_with_body() {
        assert_eq!(fmt("#project p(X):q(X).\n", 100), "#project p(X) : q(X).");
    }

    #[test]
    fn external_body_breaks_under_narrow_width() {
        // The `: body` drops to indent 4 like a rule body when the whole overflows.
        // The neck trails the head (`#external pred(X, Y) :`), the body drops to a
        // relative indent-4 block; here the body still overflows at indent 4 (its 22
        // flat cols exceed the 20 remaining), so it explodes one literal per line with
        // the `,` trailing and the `.` glued to the last — byte-identical in SHAPE to
        // `headed_rule_explodes_neck_trails_body_drops_indent_4`. Golden PINNED to the
        // actual run (the trailing-neck reuse): no trailing space — the `.` tail abuts
        // the final literal exactly as in a headed rule (idempotence note).
        let out = fmt("#external pred(X, Y) : edge(X, Z), edge(Z, Y).\n", 24);
        assert_eq!(
            out,
            "#external pred(X, Y) :\n    edge(X, Z),\n    edge(Z, Y)."
        );
    }

    // ----- #program & #heuristic -----

    #[test]
    fn program_no_params() {
        assert_eq!(fmt("#program base.\n", 100), "#program base.");
    }

    #[test]
    fn program_with_params() {
        assert_eq!(fmt("#program acid(k,t).\n", 100), "#program acid(k, t).");
    }

    #[test]
    fn program_empty_params() {
        // `#program p().` — empty parens hug.
        assert_eq!(fmt("#program p().\n", 100), "#program p().");
    }

    #[test]
    fn heuristic_weight_and_type_tail() {
        // `@` is tight (`2@1`); the `,` in the tail is spaced (`[2@1, sign]`).
        assert_eq!(
            fmt("#heuristic a(X) : b(X). [2@1,sign]\n", 100),
            "#heuristic a(X) : b(X). [2@1, sign]"
        );
    }

    #[test]
    fn heuristic_simple_weight() {
        assert_eq!(
            fmt("#heuristic a. [3,true]\n", 100),
            "#heuristic a. [3, true]"
        );
    }

    // ----- #edge -----

    #[test]
    fn edge_single_pair() {
        assert_eq!(fmt("#edge (a,b).\n", 100), "#edge (a, b).");
    }

    #[test]
    fn edge_pair_pool_semicolon_spaced() {
        assert_eq!(fmt("#edge (a,b;c,d).\n", 100), "#edge (a, b; c, d).");
    }

    #[test]
    fn edge_with_condition() {
        assert_eq!(
            fmt("#edge (u,v):reach(u,v).\n", 100),
            "#edge (u, v) : reach(u, v)."
        );
    }

    #[test]
    fn edge_multi_pair_with_condition_keeps_semicolon_spacing() {
        // The `;` between pairs must keep its space even when a `: condition` routes the
        // pairs into the trailing-neck head (concat path).
        assert_eq!(
            fmt("#edge (a,b;c,d):reach.\n", 100),
            "#edge (a, b; c, d) : reach."
        );
    }

    // ----- weak constraint, #script, doc_comment -----

    #[test]
    fn weak_constraint_flat_with_weight_priority_tail() {
        assert_eq!(
            fmt(":~ p(X), q(X). [1@2, X]\n", 100),
            ":~ p(X), q(X). [1@2, X]"
        );
    }

    #[test]
    fn weak_constraint_simple_weight_tail() {
        assert_eq!(fmt(":~ p(X). [1]\n", 100), ":~ p(X). [1]");
    }

    #[test]
    fn weak_constraint_body_breaks_neck_leads() {
        // Leading-neck like an integrity constraint (cf. the IC gallery case): the neck
        // leads, the first literal rides it, the rest hang at the neck width (3), and the
        // `[w]` tail rides after the dot. Width PINNED to 30 from the actual run: at
        // width 22 the second literal `assign(T, S2)` further explodes its args
        // (its hang line `   assign(T, S2). [1@1]` is 23 cols > 22), which muddies the
        // canonical shape; ≥23 yields the intended clean one-per-line golden.
        let out = fmt(":~ assign(T, S1), assign(T, S2). [1@1]\n", 30);
        assert_eq!(out, ":~ assign(T, S1),\n   assign(T, S2). [1@1]");
    }

    #[test]
    fn weak_constraint_body_breaks_keeps_full_tail_spacing() {
        // The task's explicit concern: a `[w@p, terms]` tail (multiple bracket elements)
        // must keep its spacing when the body BREAKS. The whole tail (`.` + the gobbled
        // `[1@2, X]`) rides after the last body literal via `leading_neck`'s `concat` —
        // the bracket interior `, ` is baked literal by `bracket_tail`, so nothing drops.
        // Width PINNED to 30 (the longer tail needs ≥26 for the clean one-per-line shape).
        let out = fmt(":~ assign(T, S1), assign(T, S2). [1@2, X]\n", 30);
        assert_eq!(out, ":~ assign(T, S1),\n   assign(T, S2). [1@2, X]");
    }

    #[test]
    fn script_wrapper_spaced_body_verbatim() {
        // `#script (lang)` spaced; the body byte-exact; `#end.` glued.
        let src = "#script (python)\ndef f():\n    return 1\n#end.\n";
        assert_eq!(
            fmt(src, 100),
            "#script (python)\ndef f():\n    return 1\n#end."
        );
    }

    // ----- hardening (golden table, depth-safety, swept Minors) -----

    #[test]
    fn family6_golden_flat_forms() {
        // Worked house-style directive forms at width 100 (flat), each pinned from a
        // run. The inputs are de-housed (no spaces, glued tails) so a regression that
        // stopped normalizing would fail RED — the output is the house form.
        for (src, want) in [
            ("#show .\n", "#show."),
            ("#defined p/1 .\n", "#defined p/1."),
            ("#const n=3. [default]\n", "#const n = 3. [default]"),
            (
                "#external p(X):q(X). [true]\n",
                "#external p(X) : q(X). [true]",
            ),
            (
                "#heuristic a:b. [2@1,sign]\n",
                "#heuristic a : b. [2@1, sign]",
            ),
            ("#edge (a,b;c,d).\n", "#edge (a, b; c, d)."),
            ("#program acid(k,t).\n", "#program acid(k, t)."),
            (":~ p. [1@2,a]\n", ":~ p. [1@2, a]"),
        ] {
            assert_eq!(fmt(src, 100), want, "golden: {src:?}");
        }
    }

    #[test]
    fn external_body_descends_deeply_without_overflow() {
        // A deeply-nested directive body must descend and RETURN on a 512 KiB stack — the
        // directive rides the SAME iterative work-list as every other construct.
        let out = crate::test_support::run_on_tiny_stack(|| {
            let n = 2000;
            let src = format!("#external {}p0{} : q.\n", "f(".repeat(n), ")".repeat(n));
            fmt(&src, 80)
        });
        assert!(
            out.starts_with("#external f("),
            "deep directive body must descend"
        );
        assert!(out.contains("p0"), "the innermost term must be reached");
    }

    #[test]
    fn include_normalizes_noncanonical_spacing() {
        // A non-canonical `#include` (extra keyword space, space before the dot,
        // padded `< id >`) normalizes — one keyword space, the `<id>` tight, the dot
        // glued. ≈-safe (same token run); the angle-bracket form AND the string form.
        assert_eq!(fmt("#include  <incmode> .\n", 100), "#include <incmode>.");
        assert_eq!(fmt("#include  \"lib.lp\" .\n", 100), "#include \"lib.lp\".");
    }

    #[test]
    fn colon_less_directives_are_bare() {
        // The colon-less `_colon_body` forms — `#external p.` and `#project p(X).`
        // carry no `:` connective, so they fall to the flat shape (keyword spaced, dot
        // glued), never a dangling `:` or trailing space.
        assert_eq!(fmt("#external p.\n", 100), "#external p.");
        assert_eq!(fmt("#project p(X).\n", 100), "#project p(X).");
    }

    #[test]
    fn external_colon_body_with_type_tail_breaks_under_narrow_width() {
        // A colon-body `#external` with a `[type]` tail at narrow width — the body
        // drops to indent 4 (trailing-neck) and the `. [false]` tail rides after the
        // last literal, its bracket spacing intact (the tail is one baked Element). The
        // `[false]` never detaches from the dot when the body explodes.
        let out = fmt("#external pred(X) : edge(X, Z), edge(Z, Y). [false]\n", 24);
        assert_eq!(
            out,
            "#external pred(X) :\n    edge(X, Z),\n    edge(Z, Y). [false]"
        );
    }

    #[test]
    fn program_params_explode_under_narrow_width() {
        // `#program`'s `(params)` ride the SHARED bracketed machinery, so an
        // over-long parameter list explodes one identifier per line with the `,`
        // trailing, exactly like a function argument list (`lower_program` → the
        // assembler's bracketed path).
        let out = fmt("#program acid(kk, tt, uu, vv).\n", 12);
        assert_eq!(out, "#program acid(\n    kk,\n    tt,\n    uu,\n    vv\n).");
    }

    #[test]
    fn edge_parens_explode_but_pairs_stay_flat() {
        // At narrow width the `#edge` parens may explode (the bracketed body drops
        // to indent 4), but the pairs stay FLAT — the pair `,` and the pool `;` are
        // baked literal spaces (mode-invariant), never break points (`lower_edge` has no
        // depth model; `lower_edge_pair`'s `,` is at depth 0). The pairs render
        // identically whether `#edge` lays out bracketed or as a trailing-neck head.
        assert_eq!(
            fmt("#edge (aaa, bbb; ccc, ddd).\n", 12),
            "#edge (\n    aaa, bbb; ccc, ddd\n)."
        );
        assert_eq!(
            fmt("#edge (aaa, bbb; ccc, ddd) : reach.\n", 12),
            "#edge (aaa, bbb; ccc, ddd) :\n    reach."
        );
    }

    #[test]
    fn doc_comment_round_trips_verbatim() {
        // A first-class statement, preserved verbatim; the emitter changes nothing.
        let src = "%*! p/1 a predicate *%\np(1).\n";
        let out = fmt(src, 100);
        assert!(out.contains("%*! p/1 a predicate *%"));
        assert!(out.contains("p(1)."));
    }
}
