//! The sole text-producing path of the emitter. The only way to
//! make leaf text is to slice the source, so a synthesized `Doc::Leaf("#minimize")`
//! is unwriteable: the source-slice-only invariant holds at the emitter boundary by construction, not by
//! discipline. A slice with an embedded newline routes to `verbatim` (byte-exact,
//! no re-indent); the empty slice (e.g. a zero-width
//! `empty_pool_item`) routes to `nil`.

use doc::{DocBuilder, NodeId};
use tree_sitter::Node;

/// Emit `node`'s verbatim source slice as one Doc leaf — the single point at which
/// leaf text enters the document. Newline-bearing slices become a `verbatim`
/// span; the empty slice becomes `nil`.
pub(super) fn token(node: Node, src: &str, b: &mut DocBuilder) -> NodeId {
    let range = node.byte_range();
    let slice = &src[range.start..range.end];
    if slice.is_empty() {
        return b.nil();
    }
    // Checked, not `as`: a silent truncating cast would corrupt the offset and
    // break the equivalence guarantee (mirrors the doc builder's `to_u32`).
    let start = u32::try_from(range.start)
        .expect("source offset exceeds u32::MAX (sources > 4 GiB are out of scope)");
    if slice.contains('\n') {
        b.verbatim(start, slice)
    } else {
        b.leaf(start, slice)
    }
}

#[cfg(test)]
mod tests {
    use super::token;
    use tree_sitter::Node;

    /// First descendant (pre-order, self included) whose `kind()` equals `kind`.
    fn find_first_by_kind<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
        if node.kind() == kind {
            return Some(node);
        }
        (0..node.child_count()).find_map(|i| {
            node.child(i)
                .and_then(|child| find_first_by_kind(child, kind))
        })
    }

    /// First leaf whose source slice equals `text`. Matching the bytes rather than
    /// the node kind keeps the test robust to how the grammar names anonymous
    /// keyword tokens.
    fn find_leaf_by_text<'t>(node: Node<'t>, src: &str, text: &str) -> Option<Node<'t>> {
        if node.child_count() == 0 {
            return (&src[node.byte_range()] == text).then_some(node);
        }
        (0..node.child_count()).find_map(|i| {
            node.child(i)
                .and_then(|child| find_leaf_by_text(child, src, text))
        })
    }

    #[test]
    fn token_emits_verbatim_source_slice_no_normalization() {
        // `#minimise` (British) must survive byte-exact — never normalized to
        // `#minimize` (the emitter does not rewrite a token's spelling).
        let src = "#minimise{}.\n";
        let tree = crate::cst::parse(src);
        let kw = find_leaf_by_text(tree.root_node(), src, "#minimise")
            .expect("the #minimise keyword leaf");
        let mut b = doc::DocBuilder::new();
        let id = token(kw, src, &mut b);
        assert_eq!(doc::render(&b.finish(id), src, 80), "#minimise");
    }

    #[test]
    fn token_routes_newline_bearing_slice_to_verbatim() {
        // A multi-line block comment routes to a verbatim span: byte-exact, with
        // no re-indent of the second line.
        let src = "%* multi\nline *%\n";
        let tree = crate::cst::parse(src);
        let bc =
            find_first_by_kind(tree.root_node(), "block_comment").expect("a block_comment node");
        let mut b = doc::DocBuilder::new();
        let id = token(bc, src, &mut b);
        assert_eq!(doc::render(&b.finish(id), src, 80), "%* multi\nline *%");
    }
}
