//! Whole-program symbol liveness for unused-declaration removal (#13105).
//!
//! `Scoping::symbol_is_unused` is reference-count based, so a declaration
//! that only references itself (`function f() { f() }`) or participates in a
//! reference cycle (`function c() { d() } function d() { c() }`) never reaches
//! zero references and survives even though no live code can reach it.
//!
//! This module computes reachability instead: roots are references that occur
//! in live-executing context (including `export { f }` specifiers, which carry
//! real references); references inside a *candidate* declaration's deferred
//! region (function bodies, side-effect-free initializers, class bodies) only
//! mark their targets live once the candidate itself is marked live. Dead
//! cycles are simply never reached — no cycle detection is needed.
//!
//! ## The gate-mirror invariant (correctness, not style)
//!
//! Candidacy here must be a SUBSET of what the removal sites in
//! `remove_unused_declaration.rs` will actually remove — cleanly, with no
//! residue. Every member of a dead cycle is removed in the same pass; if one
//! member's removal were blocked (or left residue carrying references), the
//! survivors would reference bindings that no longer exist. Concretely:
//!
//! - global gates: `unused != Keep`; skip entirely when the root scope has
//!   `ScopeFlags::DirectEval` (mirrors `can_remove_unused_declarators`; the
//!   flag propagates to the root from any direct eval in the program, so this
//!   also subsumes the per-site current-scope checks).
//! - script-mode: statements whose removal site runs at the root scope are
//!   position-ineligible (mirrors `keep_top_level_var_in_script_mode`, which
//!   compares the *visitation* scope of the removal site — for a for-init
//!   declarator that is one scope above the declarator itself).
//! - declarator inits must be dropped whole by `remove_unused_expression`:
//!   kinds with specialized handlers there can leave residue, so candidacy
//!   excludes them via the shared
//!   [`PeepholeOptimizations::expr_has_specialized_unused_handler`].
//! - class candidacy requires
//!   [`PeepholeOptimizations::classify_class_removability`] (shared with
//!   `remove_unused_class` itself) to return `RemovesClean` — removal must
//!   not bail AND must extract nothing into live code.
//!
//! ## Per-symbol candidacy: position failures force-root
//!
//! Candidacy is granted per declaration SITE, but the dead bit is consumed
//! per SYMBOL — and `var`s redeclare. A site that fails a POSITION gate is
//! not merely a non-candidate: it force-roots the symbols it binds (marks
//! them live outright), because otherwise a removable sibling site of the
//! same symbol could strip the initializer while the ineligible site keeps
//! the binding observable. Position gates:
//!
//! - export-wrapped declarations (`export var f;` carries no reference, yet
//!   importers observe the binding);
//! - script-mode bindings that live in the ROOT scope wherever their
//!   statement sits (a block `var` hoists its BINDING to the root, where
//!   other scripts observe it), plus the visitation-root mirror above;
//! - for-in/of head declarators (no removal site handles them at all);
//! - Annex B block-level function declarations in sloppy code whose binding
//!   was NOT hoisted into the var scope (the binder hoists only the first
//!   same-named function per var scope, so a later duplicate's runtime
//!   var-alias re-assignment is invisible to reference resolution);
//! - `using` declarators (removal always bails on them).
//!
//! SHAPE failures (residue-leaving init kinds, destructuring patterns,
//! `Extracts`/`Keep` classes) do NOT force-root: such a site either keeps
//! its declaration (still binding the symbol) or leaves residue whose
//! references were never deferred — they attribute to the enclosing region
//! and root the peers on their own.
//!
//! Side-effect judgments here use a context with strictly less information
//! than the peephole `TraverseCtx` (no tracked constant values). Extra
//! information only ever proves MORE expressions pure, so "pure here" implies
//! "pure at the removal site" — the direction the invariant needs.
//!
//! ## Allocation discipline
//!
//! Everything sized by the program lives in the arena, like
//! `PassDirty::dead_refs`. References are filtered at record time on
//! candidate-kind `SymbolFlags` (most references cost one flag check and no
//! storage), roots are deduplicated at record time by marking them live
//! immediately, and the edge "graph" is a flat list sorted by source symbol,
//! range-scanned via `partition_point` — no per-candidate allocations.

use std::cell::Cell;

use oxc_allocator::{Allocator, BitSet, Vec as ArenaVec};
use oxc_ast::ast::*;
use oxc_ast_visit::{
    Visit,
    walk::{
        walk_class, walk_export_default_declaration, walk_export_named_declaration,
        walk_for_statement_init, walk_for_statement_left, walk_function, walk_variable_declarator,
    },
};
use oxc_ecmascript::{
    BoundNames, GlobalContext,
    side_effects::{
        MayHaveSideEffects, MayHaveSideEffectsContext, PropertyReadSideEffects, is_pure_function,
    },
};
use oxc_semantic::{IsGlobalReference, Scoping};
use oxc_syntax::{
    reference::ReferenceId,
    scope::{ScopeFlags, ScopeId},
    symbol::{SymbolFlags, SymbolId},
};

use crate::{
    CompressOptions, CompressOptionsUnused,
    peephole::{ClassRemovability, PeepholeOptimizations},
};

/// Symbol kinds a liveness candidate can have: function/class declarations
/// and `var`/`let`/`const` bindings. A necessary (not sufficient) condition
/// for candidacy.
const CANDIDATE_KINDS: SymbolFlags = SymbolFlags::FunctionScopedVariable
    .union(SymbolFlags::BlockScopedVariable)
    .union(SymbolFlags::Class)
    .union(SymbolFlags::Function);

/// Whether the driver must recompute [`compute_dead_symbols`] after a flush:
/// a reference to a candidate-kind symbol was pruned while the symbol still
/// has remaining references — the reference-loss transition that turns a
/// live reference cycle dead. A dropped direct eval also triggers (the flag
/// refresh may clear the root flag that forces [`compute_dead_symbols`] to
/// skip). Candidacy-SHAPE transitions are deliberately NOT caught: a fold
/// can turn a specialized-kind init into a fully-removable one (e.g.
/// `(0, () => b())` into `() => b()`) without pruning any reference, so the
/// newly-dead cycle survives this run — stale toward LIVE, hence sound; the
/// cycle drops on a second minify run (a known non-idempotency, traded for
/// not re-walking on every shape-changing fold). On large inputs most
/// passes prune candidate-kind references, so this fires often; its job is
/// to skip the walk for quiet/late iterations, not to make recomputes rare.
///
/// Must be called AFTER `retain_resolved_references_excluding`: the prune
/// only shrinks per-symbol resolved-reference lists, `Reference` entries
/// survive, so the pruned references can still be inspected here.
pub fn recompute_trigger(
    scoping: &Scoping,
    options: &CompressOptions,
    dead_refs: &BitSet<'_>,
    eval_dropped: bool,
) -> bool {
    if options.unused == CompressOptionsUnused::Keep {
        return false;
    }
    if eval_dropped {
        return true;
    }
    if scoping.root_scope_flags().contains_direct_eval() {
        // `compute_dead_symbols` would skip anyway.
        return false;
    }
    dead_refs.ones().any(|idx| {
        scoping.get_reference(ReferenceId::from_usize(idx)).symbol_id().is_some_and(|symbol_id| {
            scoping.symbol_flags(symbol_id).intersects(CANDIDATE_KINDS)
                && !scoping.symbol_is_unused(symbol_id)
        })
    })
}

/// Side-effect context for use outside the peephole traversal. Mirrors the
/// `TraverseCtx` impls in `traverse_context/ecma_context.rs`, minus the
/// tracked-constant lookups (the trait defaults are strictly more
/// conservative).
struct LivenessCtx<'b> {
    scoping: &'b Scoping,
    options: &'b CompressOptions,
}

impl<'a> GlobalContext<'a> for LivenessCtx<'_> {
    fn is_global_reference(&self, ident: &IdentifierReference<'a>) -> bool {
        ident.is_global_reference(self.scoping)
    }
}

impl MayHaveSideEffectsContext<'_> for LivenessCtx<'_> {
    fn annotations(&self) -> bool {
        self.options.treeshake.annotations
    }

    fn manual_pure_functions(&self, callee: &Expression) -> bool {
        is_pure_function(callee, &self.options.treeshake.manual_pure_functions)
    }

    fn property_read_side_effects(&self) -> PropertyReadSideEffects {
        self.options.treeshake.property_read_side_effects
    }

    fn property_write_side_effects(&self) -> bool {
        self.options.treeshake.property_write_side_effects
    }

    fn unknown_global_side_effects(&self) -> bool {
        self.options.treeshake.unknown_global_side_effects
    }
}

/// Compute the set of candidate symbols no live code can reach. Bits are
/// `SymbolId::index()`; ids minted after this runs are beyond capacity and
/// read as live (`BitSet::contains` is `false` past capacity), matching the
/// `PassDirty::dead_refs` convention.
pub fn compute_dead_symbols<'a>(
    program: &Program<'a>,
    scoping: &Scoping,
    options: &CompressOptions,
    allocator: &'a Allocator,
) -> BitSet<'a> {
    let empty = || BitSet::new_in(0, allocator);
    if options.unused == CompressOptionsUnused::Keep {
        return empty();
    }
    // Mirrors `can_remove_unused_declarators` and subsumes the per-site
    // current-scope checks: any direct eval flags the root via ancestor
    // propagation (see `refresh_direct_eval_flags`).
    if scoping.root_scope_flags().contains_direct_eval() {
        return empty();
    }

    let symbols_len = scoping.symbols_len();
    let mut collector = Collector {
        ctx: LivenessCtx { scoping, options },
        is_script: program.source_type.is_script(),
        scope_depth: 0,
        for_init_depth: None,
        for_head_depth: None,
        in_export: false,
        enclosing_candidate: None,
        candidates: BitSet::new_in(symbols_len, allocator),
        live: BitSet::new_in(symbols_len, allocator),
        roots: ArenaVec::new_in(&allocator),
        edges: ArenaVec::new_in(&allocator),
    };
    collector.visit_program(program);

    let Collector { candidates, mut live, roots, mut edges, .. } = collector;
    if candidates.is_empty() {
        return empty();
    }

    // The flat edge list sorted by source symbol is the adjacency "map":
    // a live symbol's targets are one `partition_point` range scan away.
    edges.sort_unstable_by_key(|&(from, _)| from.index());

    // Roots are already marked live (record-time dedup); propagate.
    // Marking non-candidate targets live is harmless — only candidates are
    // consulted below.
    let mut worklist = roots;
    while let Some(symbol_id) = worklist.pop() {
        // Only candidates have outgoing edges by construction.
        if !candidates.contains(symbol_id.index()) {
            continue;
        }
        let start = edges.partition_point(|&(from, _)| from.index() < symbol_id.index());
        for &(_, target) in edges[start..].iter().take_while(|&&(from, _)| from == symbol_id) {
            if !live.contains(target.index()) {
                live.set_bit(target.index());
                worklist.push(target);
            }
        }
    }

    let mut dead = BitSet::new_in(symbols_len, allocator);
    for candidate in candidates.ones() {
        if !live.contains(candidate) {
            dead.set_bit(candidate);
        }
    }
    dead
}

struct Collector<'a, 'b> {
    ctx: LivenessCtx<'b>,
    is_script: bool,
    /// Scope nesting depth; 1 = the program (root) scope.
    scope_depth: usize,
    /// Depth at which a `ForStatementInit` subtree is being visited (the
    /// for's own scope is already entered). Its direct declarators' removal
    /// site runs one scope above; the exact-depth match leaves declarators
    /// nested deeper inside the init (function bodies) unadjusted.
    for_init_depth: Option<usize>,
    /// Depth at which a `ForStatementLeft` (for-in/of head) subtree is being
    /// visited. Head declarators have no removal site at all.
    for_head_depth: Option<usize>,
    /// The visited node is the `declaration` of an export statement (or a
    /// sibling declarator of one). Cleared for the declaration's subtree so
    /// declarations nested inside an exported function/class stay eligible.
    in_export: bool,
    /// Innermost candidate whose deferred region we are inside.
    enclosing_candidate: Option<SymbolId>,
    candidates: BitSet<'a>,
    /// Marked at record time for roots, during propagation for edge targets.
    live: BitSet<'a>,
    /// Deduplicated live-context references to candidate-kind symbols (plus
    /// force-rooted position-ineligible declarations); the propagation
    /// worklist.
    roots: ArenaVec<'a, SymbolId>,
    /// `(innermost enclosing candidate, referenced candidate-kind symbol)`.
    /// May contain duplicates (one entry per reference); propagation dedups
    /// via the `live` bitset.
    edges: ArenaVec<'a, (SymbolId, SymbolId)>,
}

impl Collector<'_, '_> {
    fn depth(&self) -> usize {
        self.scope_depth
    }

    /// `keep_top_level_var_in_script_mode` mirror: script-mode statements
    /// whose removal site runs at the root scope are not removable. `depth`
    /// is the visitation depth of that removal site.
    fn scope_allows_removal(&self, depth: usize) -> bool {
        !(self.is_script && depth <= 1)
    }

    /// In script mode a binding that lives in the ROOT scope is observable
    /// by other scripts (global object property / global lexical binding)
    /// no matter where its declaration statement sits: a block `var` hoists
    /// its BINDING to the root while its statement stays at removable block
    /// depth.
    fn symbol_is_script_global(&self, symbol_id: SymbolId) -> bool {
        self.is_script
            && self.ctx.scoping.symbol_scope_id(symbol_id) == self.ctx.scoping.root_scope_id()
    }

    /// Annex B.3.3: in sloppy code, a plain block-level function declaration
    /// also creates a runtime var-alias binding in the enclosing var scope.
    /// The binder models that alias by MOVING the first same-named
    /// function's binding into the var scope (accurate — references resolve
    /// to it); a plain function whose binding still sits in a sloppy non-var
    /// scope is a later duplicate (or a blocked hoist), whose runtime
    /// re-assignment of the alias is invisible to reference resolution — no
    /// such declaration is a safe candidate. Applies to sloppy non-script
    /// code too (`.cjs`), so this checks strictness, not `is_script`.
    fn is_unmodeled_annex_b_alias(&self, func: &Function<'_>, symbol_id: SymbolId) -> bool {
        // Annex B.3.3 covers plain functions only; async/generator block
        // functions are purely lexical and accurately modeled.
        if func.r#async || func.generator {
            return false;
        }
        let flags = self.ctx.scoping.scope_flags(self.ctx.scoping.symbol_scope_id(symbol_id));
        !flags.is_var() && !flags.is_strict_mode()
    }

    /// Mark a symbol live and enqueue it for propagation; deduplicated at
    /// record time via the `live` bitset. Used for references in live
    /// context and for force-rooted position-ineligible declarations.
    fn mark_live_root(&mut self, symbol_id: SymbolId) {
        if !self.live.contains(symbol_id.index()) {
            self.live.set_bit(symbol_id.index());
            self.roots.push(symbol_id);
        }
    }

    /// Shared visitor skeleton for the three declaration kinds: clear
    /// `in_export` for the subtree, track the innermost enclosing candidate
    /// across the walk, restore both.
    fn walk_declaration(&mut self, candidate: Option<SymbolId>, walk: impl FnOnce(&mut Self)) {
        let saved_export = std::mem::replace(&mut self.in_export, false);
        let saved_candidate = self.enclosing_candidate;
        if let Some(symbol_id) = candidate {
            self.candidates.set_bit(symbol_id.index());
            self.enclosing_candidate = Some(symbol_id);
        }
        walk(self);
        self.enclosing_candidate = saved_candidate;
        self.in_export = saved_export;
    }

    /// Init shapes `remove_unused_expression` fully drops when pure. Kinds
    /// with specialized handlers can leave residue (a surviving expression
    /// whose references would dangle once the cycle is removed), so they are
    /// not candidates.
    fn init_fully_removable(&self, init: Option<&Expression<'_>>) -> bool {
        match init {
            None
            | Some(Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_)) => {
                true
            }
            Some(e) => {
                !PeepholeOptimizations::expr_has_specialized_unused_handler(e)
                    && !e.may_have_side_effects(&self.ctx)
            }
        }
    }
}

impl<'a> Visit<'a> for Collector<'_, '_> {
    fn enter_scope(&mut self, _flags: ScopeFlags, _scope_id: &Cell<Option<ScopeId>>) {
        self.scope_depth += 1;
    }

    fn leave_scope(&mut self) {
        self.scope_depth -= 1;
    }

    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        if let Some(reference_id) = it.reference_id.get()
            && let Some(symbol_id) = self.ctx.scoping.get_reference(reference_id).symbol_id()
            && self.ctx.scoping.symbol_flags(symbol_id).intersects(CANDIDATE_KINDS)
        {
            match self.enclosing_candidate {
                None => self.mark_live_root(symbol_id),
                Some(from) => self.edges.push((from, symbol_id)),
            }
        }
    }

    fn visit_export_named_declaration(&mut self, it: &ExportNamedDeclaration<'a>) {
        let saved = std::mem::replace(&mut self.in_export, true);
        walk_export_named_declaration(self, it);
        self.in_export = saved;
    }

    fn visit_export_default_declaration(&mut self, it: &ExportDefaultDeclaration<'a>) {
        let saved = std::mem::replace(&mut self.in_export, true);
        walk_export_default_declaration(self, it);
        self.in_export = saved;
    }

    fn visit_for_statement_init(&mut self, it: &ForStatementInit<'a>) {
        let saved = self.for_init_depth.replace(self.depth());
        walk_for_statement_init(self, it);
        self.for_init_depth = saved;
    }

    fn visit_for_statement_left(&mut self, it: &ForStatementLeft<'a>) {
        let saved = self.for_head_depth.replace(self.depth());
        walk_for_statement_left(self, it);
        self.for_head_depth = saved;
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        let mut candidate = None;
        if it.is_declaration()
            && let Some(symbol_id) = it.id.as_ref().and_then(|id| id.symbol_id.get())
        {
            if self.in_export
                || !self.scope_allows_removal(self.depth())
                || self.symbol_is_script_global(symbol_id)
                || self.is_unmodeled_annex_b_alias(it, symbol_id)
            {
                self.mark_live_root(symbol_id);
            } else {
                candidate = Some(symbol_id);
            }
        }
        self.walk_declaration(candidate, |v| walk_function(v, it, flags));
    }

    fn visit_class(&mut self, it: &Class<'a>) {
        let mut candidate = None;
        if it.is_declaration()
            && let Some(symbol_id) = it.id.as_ref().and_then(|id| id.symbol_id.get())
        {
            if self.in_export
                || !self.scope_allows_removal(self.depth())
                || self.symbol_is_script_global(symbol_id)
            {
                self.mark_live_root(symbol_id);
            } else if matches!(
                PeepholeOptimizations::classify_class_removability(it, &self.ctx),
                ClassRemovability::RemovesClean
            ) {
                // Shape failures (`Keep` / `Extracts`) do NOT force-root:
                // the class survives whole, or its extraction's references
                // were never deferred (non-candidates attribute outward).
                candidate = Some(symbol_id);
            }
        }
        self.walk_declaration(candidate, |v| walk_class(v, it));
    }

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        // The for-init removal site (`handle_for_statement`'s retain) runs
        // while the traversal sits at the scope CONTAINING the for statement
        // — one level above this visit, since the walk has already entered
        // the for's own scope. Mirror its script-root gate at that depth.
        let statement_depth =
            if self.for_init_depth == Some(self.depth()) { self.depth() - 1 } else { self.depth() };
        let site_removable = !self.in_export
            && self.for_head_depth != Some(self.depth())
            && !it.kind.is_using()
            && self.scope_allows_removal(statement_depth);
        let mut candidate = None;
        if site_removable {
            if let BindingPattern::BindingIdentifier(id) = &it.id
                && let Some(symbol_id) = id.symbol_id.get()
            {
                if self.symbol_is_script_global(symbol_id) {
                    self.mark_live_root(symbol_id);
                } else if self.init_fully_removable(it.init.as_ref()) {
                    candidate = Some(symbol_id);
                }
            }
        } else {
            // Position-ineligible site: force-root every symbol it binds
            // (see the module doc). Shape failures land in the `if` arm
            // above and deliberately do not force-root.
            it.id.bound_names(&mut |ident| {
                if let Some(symbol_id) = ident.symbol_id.get() {
                    self.mark_live_root(symbol_id);
                }
            });
        }
        self.walk_declaration(candidate, |v| walk_variable_declarator(v, it));
    }
}

/// Debug backstop for the residue path in `handle_variable_declaration`:
/// when a DEAD-marked declarator's initializer survives as an expression
/// statement, none of its references may resolve to a dead-marked symbol.
/// Residue from a dead-cycle member can only contain references that were
/// attributed outward as roots (hence live) — candidacy excludes
/// residue-leaving init kinds precisely so their references are never
/// deferred. This is the declarator-side twin of the extracts-nothing
/// `debug_assert` at the class removal site, guarding the one direction a
/// future pass could silently break: moving an expression that carries a
/// dead-region reference into a candidate's init. Sound residue exists
/// (single-use inlining rewrites a dead declarator's init to a live call),
/// so only dead-REFERENCING residue is a violation.
#[cfg(debug_assertions)]
pub fn debug_assert_no_dead_references(
    expr: &Expression<'_>,
    scoping: &Scoping,
    dead_symbols: &BitSet<'_>,
) {
    struct Checker<'b, 'c> {
        scoping: &'b Scoping,
        dead_symbols: &'b BitSet<'c>,
    }
    impl<'a> Visit<'a> for Checker<'_, '_> {
        fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
            if let Some(reference_id) = it.reference_id.get()
                && let Some(symbol_id) = self.scoping.get_reference(reference_id).symbol_id()
            {
                debug_assert!(
                    !self.dead_symbols.contains(symbol_id.index()),
                    "residue of a dead-marked declarator references dead-marked symbol `{}`",
                    self.scoping.symbol_name(symbol_id)
                );
            }
        }
    }
    Checker { scoping, dead_symbols }.visit_expression(expr);
}
