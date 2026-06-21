//! Shared `#[cfg(test)]` helpers. Two utilities the test suite needs in more than
//! one place: the deliberate-tiny-stack runner (normal-termination probe) and
//! the grammar-driven valid-program generator (the round-trip test's input
//! source). This file starts with the runner.

/// Run `f` on a worker thread with a deliberately tiny 512 KiB stack and return its
/// result. A recursive descent deep enough to matter overflows this stack, so a test
/// that *returns* here proves the work under test is iterative (heap), not
/// call-stack-bound. Failure surfaces two ways, both fatal to the test: an
/// ordinary panic in `f` returns `Err` from `join` (caught by the `expect`); a genuine
/// stack overflow trips the guard page and `abort()`s the whole process (so the test
/// binary dies before `join` returns — `join` does NOT recover an overflow).
pub(crate) fn run_on_tiny_stack<T, F>(f: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(f)
        .expect("spawn the tiny-stack worker")
        .join()
        .expect("the work under test returns without overflowing the tiny stack")
}

use crate::style::Style;

/// Format `src` at a given line width — the width-taking test ergonomic over the public
/// [`crate::emit::format`], which takes a full [`Style`]. `line_width` is the only style knob
/// the test corpus varies, so this builds `Style::default().with_line_width(width)` and threads
/// it through the real `format`; every other house-style field stays at its default. Routes
/// through `format`, so the test path and the public API exercise identical code.
pub(crate) fn format_at_width(src: &str, width: usize) -> String {
    crate::emit::format(src, &Style::default().with_line_width(width))
}

use proptest::collection::vec;
use proptest::prelude::*;

/// A bounded strategy producing syntactically-valid clingo programs (error-free by
/// construction) — the grammar-driven input source for the `≈` round-trip test.
/// Conservative on purpose: it emits the forms the formatter must keep `≈` — atoms,
/// classical negation, argument lists, nested function terms, intervals, tuples,
/// argument pools AND tuple pools (`;`-separated, length VARYING),
/// (bare and parenthesized) arithmetic, unary minus, abs, `not`-literals, body
/// conditional literals (`elem : cond`, incl. the empty-condition `elem :`
/// edge, always trailing), disjunctive heads (`;`- AND `|`-separated, the
/// separator preserved as authored), body aggregates (`#count`/`#sum`/`#min`/
/// `#max`, an optional `>= n` bound, shallow `term` / `term : condition` elements),
/// rules / constraints, theory atoms (theory terms, functions, tuples,
/// lists, sets, the always-spaced operators, and the `op term` upper), and directives
/// (signatures, `#const`, both `#external` shapes, `#show` term-with-body,
/// `#edge`, `#program`, the weak constraint with a FIXED and a VARYING-length tail, and
/// `#script`'s verbatim body) — using
/// prefixed identifiers (`p0`/`c0`/`V0`/`f0`/`t0`/`h0`/`k0`/`W0`) so no generated name
/// collides with a keyword, and confining the ambiguity-prone forms (intervals) to
/// argument position. COMMENTS are
/// covered by a separate, deliberately-shallow [`commented_program`]: deep terms AND
/// comments together overflow the test-thread stack during proptest *generation* (an
/// orthogonal generator-depth limit), and deep nesting is already covered by the
/// tiny-stack tests, so the two nets are kept apart. The caller's `prop_assume!` drops
/// any residual non-parse, so a generator gap can never mask a real `≈` failure.
pub(crate) fn valid_program() -> impl Strategy<Value = String> {
    vec(statement(), 1..4).prop_map(|stmts| {
        let mut s = stmts.join("\n");
        s.push('\n');
        s
    })
}

/// A bounded strategy producing comment-bearing programs for the comment
/// round-trip net: SHALLOW statements decorated with leading / trailing / standalone /
/// blank-detached comments. Shallow on purpose so generation and parsing stay within the
/// test-thread stack (the deep statement FORMS are `valid_program`'s job). The point is
/// comment re-injection AND the comment-sequence ORDER that the detached-comment reorder
/// bug violated.
pub(crate) fn commented_program() -> impl Strategy<Value = String> {
    vec(commented_item(), 1..6).prop_map(|items| {
        let mut s = items.join("\n");
        s.push('\n');
        s
    })
}

/// One element of a [`commented_program`]: a shallow statement optionally decorated with
/// a leading and/or trailing comment, OR a standalone comment line, OR a standalone
/// comment followed by a blank line (the blank-DETACH case). Comment text is
/// prefixed alphanumerics (line-comment form `% …`, never a `%*` block) so it never reads
/// as code.
fn commented_item() -> impl Strategy<Value = String> {
    prop_oneof![
        8 => (
            proptest::option::of("% l[a-z0-9 ]{0,6}"),
            shallow_statement(),
            proptest::option::of("% t[a-z0-9 ]{0,6}"),
        )
            .prop_map(|(lead, stmt, trail)| {
                let mut out = String::new();
                if let Some(l) = lead {
                    out.push_str(l.trim_end());
                    out.push('\n');
                }
                out.push_str(&stmt);
                if let Some(t) = trail {
                    out.push(' ');
                    out.push_str(t.trim_end());
                }
                out
            }),
        2 => "% s[a-z0-9 ]{0,6}".prop_map(|c| c.trim_end().to_string()),
        // A trailing newline → a blank line once items are `\n`-joined, detaching this
        // comment from the element below it.
        1 => "% d[a-z0-9 ]{0,6}".prop_map(|c| format!("{}\n", c.trim_end())),
    ]
}

/// A shallow, error-free statement for [`commented_program`]: a fact, a small rule, a
/// `#show`, or an aggregate-bodied rule (the `{ }` gives a closer, so a comment inside it
/// can exercise the dangling path). No deep terms — those are `valid_program`'s job.
fn shallow_statement() -> impl Strategy<Value = String> {
    prop_oneof![
        (0u32..8).prop_map(|n| format!("p{n}.")),
        (0u32..8, 0u32..8).prop_map(|(a, b)| format!("p{a} :- p{b}.")),
        (0u32..8, 0u32..8, 0u32..8).prop_map(|(a, b, c)| format!("p{a} :- p{b}, p{c}.")),
        (0u32..8).prop_map(|n| format!("#show p{n}/1.")),
        (0u32..8, 0u32..8).prop_map(|(a, b)| format!("p{a} :- #count{{ p{b}; q{a} }}.")),
    ]
}

/// A bounded strategy producing comment-bearing programs whose comments sit at INTERNAL
/// token boundaries — the comment-TRANSPOSITION trigger family that [`commented_program`]
/// (which only decorates whole statements) never reaches. Each item splices a leading
/// comment on its own line immediately before an anonymous token AND a trailing comment
/// riding that token, for every token class a leading comment can lead (`,` `;` `|` `:-`
/// `:~` `:` `{` `(` `[` `=` `@`), plus a bounded multi-boundary "torture" statement. The
/// underlying statements are SHALLOW and in-grammar (each template is parser-validated;
/// deep forms are [`valid_program`]'s job), so generation is error-free by construction —
/// the consuming net's `prop_assume!` is a backstop that essentially never fires. The point
/// is the placement the transposition bug lived in: the leading comment must keep leading
/// and the trailing must keep trailing across a reformat, at every width.
pub(crate) fn intra_comment_program() -> impl Strategy<Value = String> {
    vec(intra_comment_item(), 1..4).prop_map(|items| {
        let mut s = items.join("\n");
        s.push('\n');
        s
    })
}

/// Splice a transposition trigger around `token`: a leading comment on its own line
/// (`%b<cls><n>`) immediately before `token`, and a trailing comment (`%a<cls><n>`) riding
/// `token` on the same line — the exact shape the transposition bug reordered. `before` /
/// `after` are the surrounding shallow, in-grammar statement fragments. `cls` is a one-letter
/// witness code UNIQUE to the token class (so a swap is visible in the `≈` witness and the
/// coverage test can confirm the class fired); `n` varies the comments across statements.
fn splice_trigger(before: &str, token: &str, after: &str, cls: &str, n: u32) -> String {
    format!("{before}\n%b{cls}{n}\n{token} %a{cls}{n}\n{after}")
}

/// One [`intra_comment_program`] item: a shallow statement carrying a transposition trigger
/// at one internal token boundary (one arm per token class), or the multi-boundary torture
/// statement. Each arm's `(before, token, after)` triple makes the class it exercises legible,
/// and the per-class `cls` code keeps every class's witness distinct.
fn intra_comment_item() -> impl Strategy<Value = String> {
    prop_oneof![
        // `,` — body-literal separator
        (0u32..8, 0u32..8, 0u32..8).prop_map(|(h, a, b)| splice_trigger(
            &format!("h{h} :- p{a}"),
            ",",
            &format!("p{b}."),
            "c",
            a
        )),
        // `;` — body-aggregate element separator
        (0u32..8, 0u32..8).prop_map(|(a, b)| splice_trigger(
            &format!("h{a} :- #sum{{ c{a} : p{b}"),
            ";",
            &format!("c{b} : p{a} }} >= 0."),
            "s",
            a
        )),
        // `|` — head-disjunction separator
        (0u32..8, 0u32..8, 0u32..8).prop_map(|(a, b, c)| splice_trigger(
            &format!("p{a}"),
            "|",
            &format!("p{b} :- r{c}."),
            "d",
            a
        )),
        // `:-` — rule neck
        (0u32..8, 0u32..8).prop_map(|(a, b)| splice_trigger(
            &format!("a{a}"),
            ":-",
            &format!("b{b}."),
            "n",
            a
        )),
        // `:~` — weak-constraint body separator (carrying the `[w@p]` tail)
        (0u32..8, 0u32..8, 0u32..5, 0u32..5).prop_map(|(a, b, w, p)| splice_trigger(
            &format!(":~ b{a}"),
            ";",
            &format!("c{b}. [{w}@{p}]"),
            "w",
            a
        )),
        // `:` — conditional-literal colon
        (0u32..8, 0u32..8, 0u32..8).prop_map(|(a, b, c)| splice_trigger(
            &format!("p{a} :- q{b}"),
            ":",
            &format!("r{c}."),
            "j",
            b
        )),
        // `{` — aggregate brace opener
        (0u32..8, 0u32..8).prop_map(|(a, b)| splice_trigger(
            &format!("h{a} :- #count"),
            "{",
            &format!("c{a} : p{b} }} >= 1."),
            "o",
            a
        )),
        // `(` — function-application paren
        (0u32..8, 0u32..8, 0u32..8).prop_map(|(h, a, b)| splice_trigger(
            &format!("h{h} :- p{a}"),
            "(",
            &format!("q{b})."),
            "p",
            a
        )),
        // `[` — weak-constraint bracket opener
        (0u32..8, 0u32..5, 0u32..5).prop_map(|(a, w, p)| splice_trigger(
            &format!(":~ b{a}."),
            "[",
            &format!("{w} @ {p}]"),
            "k",
            a
        )),
        // `@` — weight/priority infix
        (0u32..8, 0u32..5, 0u32..5).prop_map(|(a, w, p)| splice_trigger(
            &format!(":~ b{a}. [{w}"),
            "@",
            &format!("{p}]"),
            "t",
            a
        )),
        // `=` — `#const` infix
        (0u32..8, 0u32..50).prop_map(|(a, v)| splice_trigger(
            &format!("#const c{a}"),
            "=",
            &format!("{v}."),
            "e",
            a
        )),
        // the bounded multi-boundary torture statement (the example.lp shape)
        intra_torture_statement(),
    ]
}

/// The bounded "torture" statement: ONE statement carrying transposition triggers at FOUR
/// internal boundaries at once — the rule neck `:-`, an aggregate brace opener `{`, a
/// conditional `:`, and an element separator `;` — the parametric form of clingofmt's
/// `example.lp` nested-drift region. Each trigger uses an `x`-prefixed witness code so the
/// torture's comments stay distinct from the single-class arms'. The strongest stress for
/// re-injection ordering and idempotence together.
fn intra_torture_statement() -> impl Strategy<Value = String> {
    (0u32..8, 0u32..8, 0u32..8, 0u32..8).prop_map(|(h, a, b, c)| {
        format!(
            "a{h}\n%bxn{h}\n:- %axn{h}\n#sum\n%bxo{h}\n{{ %axo{h}\nc{a}\n%bxj{h}\n: %axj{h}\np{b}\n%bxs{h}\n; %axs{h}\nc{c} : p{a} }} >= 0."
        )
    })
}

fn statement() -> impl Strategy<Value = String> {
    prop_oneof![
        atom().prop_map(|h| format!("{h}.")),
        (atom(), body()).prop_map(|(h, b)| format!("{h} :- {b}.")),
        body().prop_map(|b| format!(":- {b}.")),
        // Heads (disjunction): a disjunctive head as a fact and as a rule.
        // The separator (`;` or `|`) is PRESERVED as authored — the `≈` certificate
        // compares the anonymous separator token, so a `;`↔`|` normalization is caught; the
        // generator emits both so the net exercises each.
        disjunctive_head().prop_map(|h| format!("{h}.")),
        (disjunctive_head(), body()).prop_map(|(h, b)| format!("{h} :- {b}.")),
        // A theory atom as a fact and as a rule head — exercises the theory
        // term interior (functions, tuples, lists, sets, the always-spaced operators)
        // and the `op term` upper bound through the `≈` round trip.
        theory_atom().prop_map(|t| format!("{t}.")),
        (theory_atom(), body()).prop_map(|(t, b)| format!("{t} :- {b}.")),
        // Directives — the forms whose spacing the formatter now lays out.
        directive(),
    ]
}

/// A disjunctive head (`disjunction`, the head form): 2..3 atom disjuncts
/// joined by a SINGLE separator — either `;` or `|`, chosen once per head and PRESERVED
/// verbatim through the round trip (the `≈` certificate compares the anonymous separator
/// token, so a `;`↔`|` normalization is caught). Disjuncts are atoms (incl. classical
/// negation and argument pools, both in-grammar inside a disjunct); bounded, so generation
/// stays shallow.
fn disjunctive_head() -> impl Strategy<Value = String> {
    (vec(atom(), 2..4), prop_oneof![Just("; "), Just(" | ")]).prop_map(|(ds, sep)| ds.join(sep))
}

/// A bounded directive strategy — the forms whose spacing changed, so the
/// `≈` net exercises directive layout. Each is error-free by construction: the
/// signatures / names use prefixed identifiers (`p`/`c`/`e`/`m`) that never collide
/// with a keyword; `#external` takes a `symbolic_atom` (so `atom()`'s classical-negation
/// form is in-grammar), `#show` takes a `term` (a bare `p0` is a nullary `function`, a
/// `-p0(a)` a `unary_operation`); the weak constraint carries a `[weight]` tail; the
/// `#script` exercises the `(lang)` wrapper plus the verbatim `code` body through the
/// round trip.
fn directive() -> impl Strategy<Value = String> {
    prop_oneof![
        (0u32..8, 0u32..4).prop_map(|(n, a)| format!("#show p{n}/{a}.")),
        (0u32..8, 0u32..4).prop_map(|(n, a)| format!("#defined p{n}/{a}.")),
        (0u32..8).prop_map(|n| format!("#const c{n} = {n}.")),
        atom().prop_map(|a| format!("#external {a}.")),
        (atom(), body()).prop_map(|(a, bd)| format!("#external {a} : {bd}.")),
        (atom(), body()).prop_map(|(a, bd)| format!("#show {a} : {bd}.")),
        (0u32..4, 0u32..4).prop_map(|(u, v)| format!("#edge (e{u}, e{v}).")),
        (0u32..8).prop_map(|n| format!("#program m{n}.")),
        body().prop_map(|bd| format!(":~ {bd}. [1@1]")),
        // Weak constraint: a VARYING-length tail. `[w@p]` and
        // `[w@p, t1, …, tk]` with k in 0..5 — a long tuple OVERFLOWS at narrow net widths,
        // exercising the element-explosion path. (The fixed short form
        // above is kept.)
        (body(), weak_tail()).prop_map(|(bd, tail)| format!(":~ {bd}. {tail}")),
        // `#script` through the ≈ net — `(python)` wrapper + verbatim `code` body
        // (no `#`, so the `code` token never collides with the `#end` terminator).
        (0u32..8).prop_map(|n| format!("#script (python)\nx = {n}\n#end.")),
    ]
}

/// The `[weight@priority, t1, …, tk]` tail of a weak constraint, with the tuple
/// length k VARYING over 0..5 — a long tail OVERFLOWS at narrow net widths and exercises
/// the element-explosion layout path. The tuple terms are simple leaves
/// (number / constant / variable); they need not be safe (variable safety is a grounding
/// concern, not a parse one — these inputs only need to PARSE error-free).
fn weak_tail() -> impl Strategy<Value = String> {
    (0u32..40, 0u32..5, vec(tail_term(), 0..5)).prop_map(|(w, p, terms)| {
        if terms.is_empty() {
            format!("[{w}@{p}]")
        } else {
            format!("[{w}@{p}, {}]", terms.join(", "))
        }
    })
}

/// A simple leaf term for a weak-constraint tail tuple (number / constant / variable).
fn tail_term() -> impl Strategy<Value = String> {
    prop_oneof![
        (0u32..40).prop_map(|n| n.to_string()),
        (0u32..6).prop_map(|n| format!("c{n}")),
        (0u32..6).prop_map(|n| format!("V{n}")),
    ]
}

/// A theory atom `&name { elem; … } [op term]`. Theory operators are emitted
/// ALREADY SPACED (the round trip checks the formatter KEEPS them spaced, never fusing
/// `+ +`→`++`); names are prefixed (`t`/`h`/`k`/`W`) so none collides with a keyword.
fn theory_atom() -> impl Strategy<Value = String> {
    (
        0u32..4,
        vec(theory_term(), 1..3),
        proptest::option::of(theory_upper()),
    )
        .prop_map(|(n, elems, upper)| {
            let mut s = format!("&t{n} {{ {} }}", elems.join("; "));
            if let Some(u) = upper {
                s.push_str(&u);
            }
            s
        })
}

/// The optional `op term` upper bound of a theory atom (`} <= 5`).
fn theory_upper() -> impl Strategy<Value = String> {
    (theory_op(), theory_term()).prop_map(|(op, t)| format!(" {op} {t}"))
}

/// A theory operator, used pre-spaced in generated theory terms.
fn theory_op() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("+".to_string()),
        Just("-".to_string()),
        Just("*".to_string()),
        Just("<=".to_string()),
        Just(">=".to_string()),
    ]
}

/// A recursively-nested theory term — leaves (number / constant / variable) plus theory
/// functions, tuples, lists, sets, and spaced binary/unary operator expressions. Bounded
/// by `prop_recursive` so generation terminates.
fn theory_term() -> BoxedStrategy<String> {
    let leaf = prop_oneof![
        (0u32..50).prop_map(|n| n.to_string()),  // number
        (0u32..4).prop_map(|n| format!("k{n}")), // theory constant
        (0u32..4).prop_map(|n| format!("W{n}")), // theory variable
    ];
    leaf.prop_recursive(3, 24, 2, |inner| {
        prop_oneof![
            // theory function  h(t, ...)
            (0u32..4, vec(inner.clone(), 1..3))
                .prop_map(|(n, a)| format!("h{n}({})", a.join(", "))),
            // theory tuple / list / set
            vec(inner.clone(), 1..3).prop_map(|ts| format!("({})", ts.join(", "))),
            vec(inner.clone(), 1..3).prop_map(|ts| format!("[{}]", ts.join(", "))),
            vec(inner.clone(), 1..3).prop_map(|ts| format!("{{{}}}", ts.join(", "))),
            // spaced binary theory operator  a op b
            (inner.clone(), theory_op(), inner.clone())
                .prop_map(|(a, op, b)| format!("{a} {op} {b}")),
            // spaced unary theory operator  op a
            (theory_op(), inner.clone()).prop_map(|(op, a)| format!("{op} {a}")),
        ]
    })
    .boxed()
}

/// A rule / constraint / directive body: 1..4 comma-separated literals, OR a body that
/// ENDS in a conditional literal (the `conditional_literal`). That conditional
/// literal is ALWAYS LAST: the grammar makes its condition greedily absorb the comma-
/// separated tail (`b : c, d` parses as `b` conditioned on `c, d`), so a SECOND conditional
/// (`b : c, d : e`) and an empty condition before a comma (`b :, c`) are BOTH parse errors
/// — last position is the only error-free placement. 0..3 plain literals may precede it.
/// Shared by [`statement`]'s rules/constraints, the disjunctive-head rules, and
/// [`directive`]'s `#external` / `#show` colon-bodies and weak constraint (all verified
/// error-free with a trailing conditional).
fn body() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => vec(literal(), 1..4).prop_map(|ls| ls.join(", ")),
        1 => (vec(literal(), 0..3), conditional_literal()).prop_map(|(mut ls, cond)| {
            ls.push(cond);
            ls.join(", ")
        }),
    ]
}

/// A body literal: a plain literal (atom / `not` atom) or a body aggregate. The `3 => …,
/// 1 => …` split reproduces the original even atom : not-atom : aggregate = 1 : 1 : 1
/// distribution ([`plain_literal`] is itself an even atom / not-atom split).
fn literal() -> impl Strategy<Value = String> {
    prop_oneof![3 => plain_literal(), 1 => aggregate()]
}

/// A plain body literal — an atom or its default negation `not atom` (NO aggregate).
/// Factored out so [`conditional_literal`]'s element and condition can reuse it: a
/// conditional literal's condition must be plain literals (`b : #count{…}` is a parse
/// error), and its element is a plain literal too.
fn plain_literal() -> impl Strategy<Value = String> {
    prop_oneof![atom(), atom().prop_map(|a| format!("not {a}"))]
}

/// A body conditional literal `elem : cond` (the `conditional_literal` form).
/// Wired only as [`body`]'s optional TRAILING element (never a freely-joined [`literal`]
/// arm), because the condition greedily absorbs the comma tail (see [`body`]). The element
/// is a plain literal; the condition is a comma-list of 1..3 plain literals, OR EMPTY
/// (`elem :`, the totality edge — error-free only in last position, which the [`body`]
/// placement guarantees). The conditional `:` element layout was an element-explosion
/// bug class, so fuzzing it at narrow net widths is high-value.
fn conditional_literal() -> impl Strategy<Value = String> {
    prop_oneof![
        // The empty-condition edge `elem :` (the totality edge), 1/4 of the time.
        1 => plain_literal().prop_map(|elem| format!("{elem} :")),
        // `elem : c1, …, ck` with a 1..3-literal condition.
        3 => (plain_literal(), vec(plain_literal(), 1..3))
            .prop_map(|(elem, cs)| format!("{elem} : {}", cs.join(", "))),
    ]
}

/// A body aggregate (`#count`/`#sum`/`#min`/`#max`) with an optional `>= n` bound — the
/// forms the `≈` net did not previously generate. Elements are SHALLOW (a bare
/// `c{n}` term, or `c{a} : p{b}` with a single-literal condition): deep terms are
/// `term()`'s job, and keeping the elements shallow holds proptest *generation* within
/// the test-thread stack while still exercising the aggregate's own spacing (the
/// `#op{`-abutting keyword, the spaced braces, the element `;`/`:` separators, the
/// `} >= n` bound). Prefixed identifiers (`c`/`p`) never collide with a keyword. Wired
/// into [`literal()`], so an aggregate appears wherever a body literal can — rule /
/// constraint bodies and the `#external` / `#show` / weak-constraint colon-bodies.
fn aggregate() -> impl Strategy<Value = String> {
    (
        prop_oneof![Just("#count"), Just("#sum"), Just("#min"), Just("#max")],
        vec(agg_element(), 1..3),
        proptest::option::of((0u32..5).prop_map(|n| format!(" >= {n}"))),
    )
        .prop_map(|(op, elems, bound)| {
            format!(
                "{op}{{ {} }}{}",
                elems.join("; "),
                bound.unwrap_or_default()
            )
        })
}

/// One shallow aggregate element: a bare term `c{n}`, or `c{a} : p{b}` (a term with a
/// single-literal condition). Both are in-grammar `body_aggregate_element` forms.
fn agg_element() -> impl Strategy<Value = String> {
    prop_oneof![
        (0u32..6).prop_map(|n| format!("c{n}")),
        (0u32..6, 0u32..6).prop_map(|(a, b)| format!("c{a} : p{b}")),
    ]
}

fn atom() -> impl Strategy<Value = String> {
    prop_oneof![
        pred_name(),
        (pred_name(), arg_list()).prop_map(|(p, a)| format!("{p}({a})")),
        // classical negation `-p(...)` — the neg abuts the name (tight)
        (pred_name(), arg_list()).prop_map(|(p, a)| format!("-{p}({a})")),
    ]
}

/// An argument list: comma-separated args, OR an argument POOL of 2..4 `;`-separated
/// segments (the `pool`). The pool length now VARIES (was a fixed 2-segment
/// `a; b`), so a wide pool overflows at narrow net widths; the `;` separators are
/// author-significant (the `≈` certificate compares them, and they are never `,`), so the
/// formatter must preserve every one.
fn arg_list() -> impl Strategy<Value = String> {
    prop_oneof![
        vec(arg(), 1..4).prop_map(|ts| ts.join(", ")),
        vec(arg(), 2..5).prop_map(|ts| ts.join("; ")),
    ]
}

/// A term, or an interval (`1..3`) — intervals are confined to argument position,
/// where clingo's term grammar accepts them unambiguously.
fn arg() -> impl Strategy<Value = String> {
    prop_oneof![
        term(),
        (0u32..50, 0u32..50).prop_map(|(a, b)| format!("{a}..{b}")),
    ]
}

fn pred_name() -> impl Strategy<Value = String> {
    (0u32..8).prop_map(|n| format!("p{n}"))
}

fn arith_op() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("+".to_string()),
        Just("-".to_string()),
        Just("*".to_string()),
        Just("/".to_string()),
        Just("**".to_string()), // exercises the `*`/`**` seam region
    ]
}

/// A recursively nested term (no intervals; those live at `arg`). Bounded by
/// `prop_recursive` so generation terminates.
fn term() -> BoxedStrategy<String> {
    let leaf = prop_oneof![
        (0u32..100).prop_map(|n| n.to_string()), // number
        (0u32..8).prop_map(|n| format!("c{n}")), // constant
        (0u32..6).prop_map(|n| format!("V{n}")), // variable
    ];
    leaf.prop_recursive(4, 48, 3, |inner| {
        prop_oneof![
            // function term  f(t, ...)
            (0u32..8, vec(inner.clone(), 1..3))
                .prop_map(|(n, args)| format!("f{n}({})", args.join(", "))),
            // parenthesized arithmetic  (t op t)
            (inner.clone(), arith_op(), inner.clone())
                .prop_map(|(a, op, b)| format!("({a} {op} {b})")),
            // tuple  (t,) / (t, t)
            vec(inner.clone(), 1..3).prop_map(|ts| {
                if ts.len() == 1 {
                    format!("({},)", ts[0])
                } else {
                    format!("({})", ts.join(", "))
                }
            }),
            // tuple POOL  (t; t; …)  — a `;`-separated pool in term position,
            // always nested in an argument (a bare pool body literal `:- (a; b).` is a
            // parse error; `term` only ever appears inside an argument, so the context is
            // always in-grammar). 2..3 segments; the `;` separators are preserved by `≈`.
            vec(inner.clone(), 2..4).prop_map(|ts| format!("({})", ts.join("; "))),
            // unary minus  -t  (abuts its operand)
            inner.clone().prop_map(|t| format!("-{t}")),
            // bare binary arithmetic  t op t  (exercises depth-based op spacing)
            (inner.clone(), arith_op(), inner.clone())
                .prop_map(|(a, op, c)| format!("{a} {op} {c}")),
            // abs  |t|  (a tight bracket-pair, +depth)
            inner.clone().prop_map(|t| format!("|{t}|")),
        ]
    })
    .boxed()
}
