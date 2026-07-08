//! [`multi_vec!`] macro, generating a named struct-of-arrays table type wrapping a [`MultiVec`].
//!
//! The only unsafe code the macro generates is the `Fields` impl:
//! a thin translation layer between untyped field pointers and the typed field values /
//! references, consisting purely of pointer casts and calls to helper functions in the
//! `utils` module - no arithmetic, no allocation, and no control flow.
//! All the logic (allocation, layout, pointer arithmetic, bounds checks, clone / drop
//! loops) lives in `MultiVec` and the `utils` module, in plain Rust code.
//!
//! [`MultiVec`]: super::MultiVec

/// Create a struct-of-arrays table type.
///
/// Generates:
///
/// * The plain struct itself (e.g. `Scope`), which is what `push` takes.
/// * A table type (e.g. `ScopeTable`) wrapping a `MultiVec` (private), which stores the struct's fields
///   as struct-of-arrays in a single allocation, indexed by a typed index (e.g. `ScopeId`).
/// * A shared-reference view (e.g. `ScopeRef`) returned by `get`, with a `&T` field
///   per struct field.
/// * An mutable-reference view (e.g. `ScopeMut`) returned by `get_mut`, with a `&mut T`
///   field per struct field.
/// * Slice views (e.g. `ScopeSlices` / `ScopeSlicesMut`) returned by `slices` / `slices_mut`,
///   with an [`IndexSlice`] field per struct field, for bulk access to whole field arrays.
///
/// The view type names are derived from the struct's name: a struct `Scope` produces
/// `ScopeRef`, `ScopeMut`, `ScopeSlices`, and `ScopeSlicesMut`.
/// * Iterators: `iter` / `iter_mut` yield the reference views (e.g. `ScopeRef` / `ScopeMut`),
///   and the table implements [`IntoIterator`] (by value, by `&`, and by `&mut`), with
///   `into_iter` yielding each element as an owned struct (e.g. `Scope`). Enumerated
///   variants (`iter_enumerated` etc.) also yield each element's ID. The iterator type
///   names are derived from the table's name: a table `ScopeTable` produces
///   `ScopeTableIter`, `ScopeTableIterMut`, and `ScopeTableIntoIter`.
/// * Per-field accessor methods, e.g. `.parent_id(id)` / `.parent_id_mut(id)`.
/// * The `Fields` impl for the struct, which `MultiVec` requires.
///
/// All generated types are defined directly in the invoking scope, each with the
/// visibility given on the `table` declaration (e.g. `pub table ScopeTable<ScopeId, Scope>;`).
/// The struct is written without a visibility - it is a compile-time error otherwise.
/// Fields are always `pub`; their effective visibility is bounded by the struct's.
/// Everything else the macro generates (the impls, and the helper items only they need)
/// is hidden inside an anonymous `const` block, adding no other names to the invoking
/// scope. The macro can be invoked at module scope or inside a function.
///
/// Types which need dropping (e.g. `String`) are supported: the stored field values are
/// dropped when the table is dropped.
///
/// There must be at least one field, and at least one field must have non-zero size
/// (a table whose fields are all zero-sized types would have a zero-sized allocation).
/// Both are enforced when the table is defined, at check time:
///
/// ```compile_fail
/// # use oxc_data_structures::multi_vec::multi_vec;
/// # oxc_index::define_index_type! { struct Id = u32; }
/// // All fields are zero-sized types - error
/// multi_vec! {
///     table AllZstTable<Id, AllZst>;
///
///     struct AllZst {
///         x: (),
///         y: (),
///     }
/// }
/// ```
///
/// ```compile_fail
/// # use oxc_data_structures::multi_vec::multi_vec;
/// # oxc_index::define_index_type! { struct Id = u32; }
/// // `Empty` has no fields - error
/// multi_vec! {
///     table EmptyTable<Id, Empty>;
///
///     struct Empty {}
/// }
/// ```
///
/// # Cloning
///
/// The generated types implement `Clone` only if you derive it, like ordinary structs:
///
/// * `#[derive(Clone)]` on the `table` declaration makes the table cloneable.
///   This requires the struct to also be `Clone` (derive it too) - it is a compile-time error otherwise.
/// * `#[derive(Clone)]` on the struct requires all field types to be `Clone`, as usual.
///
/// Cloning a table clones column by column: `Copy` field types are copied bitwise
/// (one `memcpy` per field array); other field types are cloned element by element.
/// The struct's own `Clone` impl is never invoked by table cloning (elements are stored
/// as scattered field values, so there is never a whole struct value to clone).
///
/// ```compile_fail
/// # use oxc_data_structures::multi_vec::multi_vec;
/// # oxc_index::define_index_type! { struct Id = u32; }
/// // No `#[derive(Clone)]` on the `Item`struct - error
/// multi_vec! {
///     #[derive(Clone)]
///     table CloneTable<Id, Item>;
///
///     struct Item { value: u32 }
/// }
/// ```
///
/// # Debug
///
/// Likewise, the generated types implement `Debug` only if you derive it:
///
/// * `#[derive(Debug)]` on the `table` declaration makes the table and the view types
///   (e.g. `ScopeRef`) implement `Debug`. This requires the struct to also be `Debug`
///   (derive it too) - it is a compile-time error otherwise.
/// * `#[derive(Debug)]` on the struct requires all field types to be `Debug`, as usual.
///
/// The table prints as a map from each element's ID to the element, formatted through
/// the shared-reference view - e.g. `{ 0: ScopeRef { parent_id: None, ... }, 1: ... }`.
///
/// ```compile_fail
/// # use oxc_data_structures::multi_vec::multi_vec;
/// # oxc_index::define_index_type! { struct Id = u32; }
/// // No `#[derive(Debug)]` on the `Item` struct - error
/// multi_vec! {
///     #[derive(Debug)]
///     table DebugTable<Id, Item>;
///
///     struct Item { value: u32 }
/// }
/// ```
///
/// `Clone` and `Debug` are the only derives accepted on the `table` declaration.
/// Any other derive is a compile-time error:
///
/// ```compile_fail
/// # use oxc_data_structures::multi_vec::multi_vec;
/// # oxc_index::define_index_type! { struct Id = u32; }
/// multi_vec! {
///     // `PartialEq` is not supported - error
///     #[derive(Clone, PartialEq)]
///
///     // Any derive is fine on the struct - it is a plain struct
///     table PartialEqTable<Id, Item>;
///     #[derive(Clone, PartialEq)]
///     struct Item { value: u32 }
/// }
/// ```
///
/// # Dropping
///
/// The struct's own `Drop` impl (if it has one) is never invoked, for the same reason -
/// the stored field values are dropped individually. Do not implement `Drop` on the struct.
///
/// # Pluralization
///
/// The fields of the slice views are pluralized: each holds a whole column, so e.g. a
/// `parent_id` field produces a `parent_ids` slice field. By default the plural is formed
/// by appending `s`. Where that produces the wrong name (e.g. `flags` -> `flagss`),
/// specify the plural with a `#[plural(...)]` attribute on the field. The attribute may
/// appear anywhere among the field's doc comments.
///
/// # Field attributes
///
/// Fields accept only doc comments and `#[plural(...)]`. Doc comments are applied to the
/// struct's field, and to the corresponding fields of the reference views (e.g.
/// `ScopeRef` / `ScopeMut`). Any other attribute is a compile-time error - it would
/// apply only to the generated struct's field, not to the fields of the generated view
/// types:
///
/// ```compile_fail
/// # use oxc_data_structures::multi_vec::multi_vec;
/// # oxc_index::define_index_type! { struct Id = u32; }
/// multi_vec! {
///     table ItemTable<Id, Item>;
///
///     struct Item {
///         #[serde(skip)] // not supported - error
///         value: u32,
///     }
/// }
/// ```
///
/// A duplicate `#[plural(...)]` is also a compile-time error:
///
/// ```compile_fail
/// # use oxc_data_structures::multi_vec::multi_vec;
/// # oxc_index::define_index_type! { struct Id = u32; }
/// multi_vec! {
///     table ItemTable<Id, Item>;
///
///     struct Item {
///         #[plural(a)]
///         #[plural(b)] // duplicate - error
///         value: u32,
///     }
/// }
/// ```
///
/// # Example
///
/// ```
/// use oxc_data_structures::multi_vec::multi_vec;
/// # oxc_index::define_index_type! { pub struct ScopeId = u32; }
/// # #[derive(Debug, Clone, Copy)] pub struct NodeId(u32);
/// # #[derive(Debug, Clone, Copy)] pub struct ScopeFlags(u8);
///
/// multi_vec! {
///     /// SoA table of [`Scope`]s.
///     #[derive(Clone, Debug)]
///     pub table ScopeTable<ScopeId, Scope>;
///
///     /// A scope.
///     #[derive(Clone, Debug)]
///     struct Scope {
///         parent_id: Option<ScopeId>,
///         node_id: NodeId,
///         #[plural(all_flags)]
///         flags: ScopeFlags,
///     }
/// }
///
/// let mut table = ScopeTable::new();
/// let id = table.push(Scope { parent_id: None, node_id: NodeId(0), flags: ScopeFlags(0) });
/// assert_eq!(*table.parent_id(id), None);
/// assert_eq!(table.get(id).parent_id, &None);
/// assert_eq!(table.slices().parent_ids.len(), 1);
/// assert_eq!(table.slices().all_flags.len(), 1);
/// *table.parent_id_mut(id) = Some(id);
/// assert_eq!(*table.parent_id(id), Some(id));
/// let clone = table.clone();
/// assert_eq!(*clone.parent_id(id), Some(id));
/// for scope in &table {
///     assert_eq!(*scope.parent_id, Some(id));
/// }
/// let scopes: Vec<Scope> = table.into_iter().collect();
/// assert_eq!(scopes.len(), 1);
/// ```
///
/// Note: doc comments and other attributes on the struct are applied to the generated
/// struct; those on the `table` declaration are applied to the generated table type.
/// They are not applied to the other generated types, so e.g. `#[cfg(...)]` will not
/// work as expected.
///
/// [`IndexSlice`]: oxc_index::IndexSlice
#[macro_export]
macro_rules! multi_vec {
    // Entry point.
    //
    // The `table` declaration's attributes are classified first, one at a time, by the
    // `@table_attrs` rules: ordinary attributes (doc comments) pass through to the
    // generated table type; `#[derive(Clone)]` passes through too, and is also recorded
    // in `config`'s `clone` slot (the `@generate_clone` rules generate the clone machinery on it);
    // `#[derive(Debug)]` is recorded in the `debug` slot (it gates the `Debug` impls
    // `@generate` emits); and any other derive is a compile error.
    // An attribute with several derives (e.g. `#[derive(Clone, Debug)]`) is first split
    // into one attribute per derive.
    //
    // Fields are then normalized one at a time: `@normalize` matches one whole field -
    // attributes, name, and type - and builds its normalized record. The `@field`
    // rules then classify the attributes one at a time, refining the record in place:
    // doc comments into `field_comments` (emitted onto the struct field),
    // `#[plural(...)]` into `field_plural_name`, and anything else is a compile error.
    // Once every field is normalized, the `@generate` rule generates the output.
    // To add a new field attribute: add a record slot, a classify rule for it, and
    // pass the slot through the other `@field` rules.
    //
    // Attributes are captured as raw token trees, NOT `:meta` fragments: a captured
    // `:meta` fragment is opaque, so the classify rules could not tell `#[plural(...)]`
    // from `#[doc]`. (Doc comments reach the macro already lowered to `#[doc = "..."]`
    // attribute tokens, so they are matched structurally like any other attribute.)
    //
    // Normalized fields have the form
    // `{ field_name = [...] field_ty = [...] field_plural_name = [...] field_comments = [...] }`.
    // `field_plural_name` is a single token tree: either the ident from a `#[plural(...)]`
    // attribute, or `[<field_name s>]`, which `paste!` concatenates to e.g. `parent_ids`.
    (
        $(# [ $($table_attr:tt)* ])*
        $vis:vis table $table_name:ident<$key:ty, $table_item_name:ident>;

        $(#[$struct_attr:meta])*
        struct $struct_name:ident {
            $($fields:tt)*
        }
    ) => {
        $crate::multi_vec! {
            @table_attrs
            config = {
                key = [ $key ]
                vis = [ $vis ]
                table_name = [ $table_name ]
                table_attrs = []
                clone = []
                debug = []
                table_item_name = [ $table_item_name ]
                struct_name = [ $struct_name ]
                struct_attrs = [ $(#[$struct_attr])* ]
            }
            pending = [ $([ $($table_attr)* ])* ]
            fields = []
            rest = [ $($fields)* ]
        }
    };

    // Visibility on the `struct` - reject. The visibility on the `table` declaration
    // is applied to every generated type, so it is the only place visibility can be
    // declared.
    // (Must come after the entry rule above: `$_struct_vis:vis` also matches an absent
    // visibility, so this rule would otherwise match valid input too.)
    (
        $(#[$_table_attr:meta])*
        $_vis:vis table $_table_name:ident<$_key:ty, $_table_item_name:ident>;

        $(#[$_struct_attr:meta])*
        $_struct_vis:vis struct $($_rest:tt)*
    ) => {
        ::std::compile_error!(
            "declare visibility on the `table` (e.g. `pub table`), not on the `struct`"
        );
    };

    // Classify a `#[derive(Clone)]` on the `table` declaration: pass it through to the
    // generated table type, and record it in the `clone` slot - the `@generate_clone`
    // rules generate the clone machinery (a `CloneFields` impl) when the slot is full.
    // The passthrough works as a real derive: `MultiVec` implements `Clone` where the
    // fields struct implements `CloneFields`.
    // (A duplicate `#[derive(Clone)]` just passes through twice and gets rustc's
    // duplicate-impl error, like any duplicated derive; the slot is set idempotently.)
    (
        @table_attrs
        config = {
            key = $key:tt
            vis = $vis:tt
            table_name = $table_name:tt
            table_attrs = [ $($table_attrs:tt)* ]
            clone = $_clone:tt
            debug = $debug:tt
            table_item_name = $table_item_name:tt
            struct_name = $struct_name:tt
            struct_attrs = $struct_attrs:tt
        }
        pending = [
            [ derive ( Clone $(,)? ) ]
            $($pending:tt)*
        ]
        fields = $fields:tt
        rest = $rest:tt
    ) => {
        $crate::multi_vec! {
            @table_attrs
            config = {
                key = $key
                vis = $vis
                table_name = $table_name
                table_attrs = [
                    $($table_attrs)*
                    #[derive(Clone)]
                ]
                clone = [ Clone ]
                debug = $debug
                table_item_name = $table_item_name
                struct_name = $struct_name
                struct_attrs = $struct_attrs
            }
            pending = [ $($pending)* ]
            fields = $fields
            rest = $rest
        }
    };

    // Classify a `#[derive(Debug)]` on the `table` declaration: record it in the `debug`
    // slot, and do NOT pass it through. It cannot be a real derive: the view types'
    // `Debug` impls must also be conditional on it, which a derive on the table type
    // alone could not achieve. `@generate` emits the `Debug` impls only when the slot
    // is filled.
    (
        @table_attrs
        config = {
            key = $key:tt
            vis = $vis:tt
            table_name = $table_name:tt
            table_attrs = $table_attrs:tt
            clone = $clone:tt
            debug = []
            table_item_name = $table_item_name:tt
            struct_name = $struct_name:tt
            struct_attrs = $struct_attrs:tt
        }
        pending = [
            [ derive ( Debug $(,)? ) ]
            $($pending:tt)*
        ]
        fields = $fields:tt
        rest = $rest:tt
    ) => {
        $crate::multi_vec! {
            @table_attrs
            config = {
                key = $key
                vis = $vis
                table_name = $table_name
                table_attrs = $table_attrs
                clone = $clone
                debug = [ Debug ]
                table_item_name = $table_item_name
                struct_name = $struct_name
                struct_attrs = $struct_attrs
            }
            pending = [ $($pending)* ]
            fields = $fields
            rest = $rest
        }
    };

    // A 2nd `#[derive(Debug)]` on the `table` (the `debug` slot is already full) - reject.
    // (A real derive would get rustc's duplicate-impl error; the slot must produce the
    // equivalent.)
    (
        @table_attrs
        config = {
            key = $_key:tt
            vis = $_vis:tt
            table_name = $_table_name:tt
            table_attrs = $_table_attrs:tt
            clone = $_clone:tt
            debug = [ $_debug:ident ]
            table_item_name = $_table_item_name:tt
            struct_name = $_struct_name:tt
            struct_attrs = $_struct_attrs:tt
        }
        pending = [
            [ derive ( Debug $(,)? ) ]
            $($_pending:tt)*
        ]
        fields = $_fields:tt
        rest = $_rest:tt
    ) => {
        ::std::compile_error!("duplicate `#[derive(Debug)]` attribute");
    };

    // A `#[derive(...)]` with more than one derive (e.g. `#[derive(Clone, Debug)]`) -
    // split off the first derive into its own attribute, and re-queue both halves, so
    // the rules above and below classify each derive separately.
    //
    // `$first:ident`, not `:path`: an `ident` capture stays transparent, so the re-queued
    // `[ derive ( $first ) ]` still matches the literal `Clone` / `Debug` in the rules
    // above. (A `:path` capture would become an opaque fragment, which they cannot match.
    // Path derives like `serde::Serialize` fall through to the rule below and are
    // rejected - only `Clone` and `Debug` are accepted anyway.)
    (
        @table_attrs
        config = $config:tt
        pending = [
            [ derive ( $first:ident , $($more:tt)+ ) ]
            $($pending:tt)*
        ]
        fields = $fields:tt
        rest = $rest:tt
    ) => {
        $crate::multi_vec! {
            @table_attrs
            config = $config
            pending = [
                [ derive ( $first ) ]
                [ derive ( $($more)+ ) ]
                $($pending)*
            ]
            fields = $fields
            rest = $rest
        }
    };

    // Any other derive on the `table` - reject. The table type wraps a `MultiVec`, which
    // supports only `Clone` and `Debug`.
    // (Must come after the `Clone` / `Debug` rules and the splitting rule above, which
    // this rule would also match.)
    (
        @table_attrs
        config = $_config:tt
        pending = [
            [ derive ( $($derives:tt)* ) ]
            $($_pending:tt)*
        ]
        fields = $_fields:tt
        rest = $_rest:tt
    ) => {
        ::std::compile_error!(::std::concat!(
            "`multi_vec!` tables accept only `#[derive(Clone)]` and `#[derive(Debug)]`, \
             not `#[derive(",
            ::std::stringify!($($derives)*),
            ")]`",
        ));
    };

    // Any other attribute on the `table` (doc comments etc.) - pass it through to the
    // generated table type.
    // (Must come after the derive rules above: this rule would also match a derive.)
    (
        @table_attrs
        config = {
            key = $key:tt
            vis = $vis:tt
            table_name = $table_name:tt
            table_attrs = [ $($table_attrs:tt)* ]
            clone = $clone:tt
            debug = $debug:tt
            table_item_name = $table_item_name:tt
            struct_name = $struct_name:tt
            struct_attrs = $struct_attrs:tt
        }
        pending = [
            [ $($attr:tt)* ]
            $($pending:tt)*
        ]
        fields = $fields:tt
        rest = $rest:tt
    ) => {
        $crate::multi_vec! {
            @table_attrs
            config = {
                key = $key
                vis = $vis
                table_name = $table_name
                table_attrs = [ $($table_attrs)* #[$($attr)*] ]
                clone = $clone
                debug = $debug
                table_item_name = $table_item_name
                struct_name = $struct_name
                struct_attrs = $struct_attrs
            }
            pending = [ $($pending)* ]
            fields = $fields
            rest = $rest
        }
    };

    // All table attributes classified - start normalizing the fields.
    (
        @table_attrs
        config = $config:tt
        pending = []
        fields = $fields:tt
        rest = $rest:tt
    ) => {
        $crate::multi_vec! {
            @normalize
            config = $config
            fields = $fields
            rest = $rest
        }
    };

    // Start normalizing the next field: build its normalized record up front, in the
    // exact form `@generate` consumes, then refine it in place as the `@field`
    // rules classify the attributes.
    //
    // `ctx` bundles the state which the classify rules carry through unexamined.
    // `field_plural_name` starts out empty, meaning "no `#[plural(...)]` seen yet";
    // if it is still empty once all attributes are classified, a dedicated rule fills
    // in the default plural name (append `s`).
    (
        @normalize
        config = $config:tt
        fields = $fields:tt
        rest = [
            $(# [ $($field_attr:tt)* ])*
            $field_name:ident: $field_ty:ty
            $(, $($rest:tt)*)?
        ]
    ) => {
        $crate::multi_vec! {
            @field
            ctx = {
                config = $config
                fields = $fields
                rest = [ $($($rest)*)? ]
            }
            field = {
                field_name = [ $field_name ]
                field_ty = [ $field_ty ]
                field_plural_name = []
                field_plural_name_mut = []
                field_comments = []
            }
            pending = [ $([ $($field_attr)* ])* ]
        }
    };

    // Classify a doc comment (`#[doc ...]`): collect it into `field_comments`, which
    // `@generate` emits onto the struct's field.
    (
        @field
        ctx = $ctx:tt
        field = {
            field_name = $field_name:tt
            field_ty = $field_ty:tt
            field_plural_name = $field_plural_name:tt
            field_plural_name_mut = $field_plural_name_mut:tt
            field_comments = [ $($field_comments:tt)* ]
        }
        pending = [
            [ doc $($body:tt)* ]
            $($pending:tt)*
        ]
    ) => {
        $crate::multi_vec! {
            @field
            ctx = $ctx
            field = {
                field_name = $field_name
                field_ty = $field_ty
                field_plural_name = $field_plural_name
                field_plural_name_mut = $field_plural_name_mut
                field_comments = [
                    $($field_comments)*
                    #[doc $($body)*]
                ]
            }
            pending = [ $($pending)* ]
        }
    };

    // Classify a `#[plural(...)]` attribute: set the plural name.
    (
        @field
        ctx = $ctx:tt
        field = {
            field_name = $field_name:tt
            field_ty = $field_ty:tt
            field_plural_name = []
            field_plural_name_mut = []
            field_comments = $field_comments:tt
        }
        pending = [
            [ plural ( $field_plural_name:ident ) ]
            $($pending:tt)*
        ]
    ) => {
        $crate::multi_vec! {
            @field
            ctx = $ctx
            field = {
                field_name = $field_name
                field_ty = $field_ty
                field_plural_name = [ $field_plural_name ]
                field_plural_name_mut = [ [<$field_plural_name _mut>] ]
                field_comments = $field_comments
            }
            pending = [ $($pending)* ]
        }
    };

    // A 2nd `#[plural(...)]` on the same field (`field_plural_name` is already full) - reject.
    (
        @field
        ctx = $_ctx:tt
        field = {
            field_name = $_field_name:tt
            field_ty = $_field_ty:tt
            field_plural_name = [ $_field_plural_name:tt ]
            field_plural_name_mut = $_field_plural_name_mut:tt
            field_comments = $_field_comments:tt
        }
        pending = [
            [ plural $($_body:tt)* ]
            $($_pending:tt)*
        ]
    ) => {
        ::std::compile_error!("duplicate `#[plural(...)]` attribute");
    };

    // A malformed `#[plural ...]` (not a single identifier in parentheses) - reject.
    // (A well-formed `#[plural(...)]` was already consumed by the rules above.)
    (
        @field
        ctx = $_ctx:tt
        field = $_field:tt
        pending = [
            [ plural $($_body:tt)* ]
            $($_pending:tt)*
        ]
    ) => {
        ::std::compile_error!(
            "`#[plural(...)]` expects a single identifier, e.g. `#[plural(all_flags)]`"
        );
    };

    // Any other attribute - reject. It would apply only to the generated struct's field,
    // not to the fields of the generated view types, which is a footgun (`#[cfg(...)]`
    // especially so).
    (
        @field
        ctx = $_ctx:tt
        field = $_field:tt
        pending = [
            [ $($body:tt)* ]
            $($_pending:tt)*
        ]
    ) => {
        ::std::compile_error!(::std::concat!(
            "`multi_vec!` fields accept only doc comments and `#[plural(...)]` on struct fields, not `#[",
            ::std::stringify!($($body)*),
            "]`",
        ));
    };

    // All attributes classified, but no `#[plural(...)]` was given - fill in the default
    // plural name (append `s`), and re-run, so the rule below pushes the field.
    (
        @field
        ctx = $ctx:tt
        field = {
            field_name = [ $field_name:ident ]
            field_ty = $field_ty:tt
            field_plural_name = []
            field_plural_name_mut = []
            field_comments = $field_comments:tt
        }
        pending = []
    ) => {
        $crate::multi_vec! {
            @field
            ctx = $ctx
            field = {
                field_name = [ $field_name ]
                field_ty = $field_ty
                field_plural_name = [ [<$field_name s>] ]
                field_plural_name_mut = [ [<$field_name s_mut>] ]
                field_comments = $field_comments
            }
            pending = []
        }
    };

    // All attributes classified - push the completed record, and hand the remaining
    // fields back to `@normalize`.
    //
    // Must come after the fill rule above: this rule would also match a record whose
    // `field_plural_name` is still empty, so such records must be filled first.
    (
        @field
        ctx = {
            config = $config:tt
            fields = [ $($fields:tt)* ]
            rest = $rest:tt
        }
        field = $field:tt
        pending = []
    ) => {
        $crate::multi_vec! {
            @normalize
            config = $config
            fields = [
                $($fields)*
                $field
            ]
            rest = $rest
        }
    };

    // All fields normalized, but there are none - reject with a clear error.
    // (Without this rule, an empty field list fails to match any rule, producing an
    // inscrutable macro-expansion error.)
    (
        @normalize
        config = $_config:tt
        fields = []
        rest = []
    ) => {
        ::std::compile_error!("`multi_vec!` requires at least one field");
    };

    // All fields normalized - generate the output.
    // (Must come after the empty-fields rule above, which would also match here.)
    (
        @normalize
        config = {
            key = [ $key:ty ]
            vis = $vis:tt
            table_name = $table_name:tt
            table_attrs = $table_attrs:tt
            clone = $clone:tt
            debug = $debug:tt
            table_item_name = $table_item_name:tt
            struct_name = $struct_name:tt
            struct_attrs = $struct_attrs:tt
        }
        fields = $fields:tt
        rest = []
    ) => {
        $crate::multi_vec! {
            @generate
            config = {
                key = [ $key ]
                id = [ [<$key:snake>] ]
                vis = $vis
                table_name = $table_name
                table_attrs = $table_attrs
                clone = $clone
                debug = $debug
                table_item_name = $table_item_name
                struct_name = $struct_name
                struct_attrs = $struct_attrs
            }
            fields = $fields
        }
    };

    // Generate the output: the struct, the table type, the view types, the iterators, and all their impls.
    (
        @generate
        config = {
            key = [ $key:ty ]
            id = [ $id:tt ]
            vis = [ $vis:vis ]
            table_name = [ $table_name:ident ]
            table_attrs = [ $(#[$table_attr:meta])* ]
            // `Clone` if the `table` declaration had `#[derive(Clone)]`, empty otherwise.
            // Forwarded to the `@generate_clone` invocation inside the generated `const` block,
            // which generates the clone machinery when the slot is full.
            clone = $clone:tt
            // `Debug` if the `table` declaration had `#[derive(Debug)]`, empty otherwise.
            // Gates the `$( ... )?` groups below which emit the `Debug` impls.
            debug = [ $($debug:ident)? ]
            table_item_name = [ $table_item_name:ident ]
            struct_name = [ $struct_name:ident ]
            struct_attrs = [ $(#[$struct_attr:meta])* ]
        }
        fields = [ $(
            {
                field_name = [ $field_name:ident ]
                field_ty = [ $field_ty:ty ]
                // `:tt` not `:ident`. The default plural name is `[<$field_name s>]` -
                // unresolved `paste!` syntax (a single bracketed token tree), which only
                // becomes an identifier when `paste!` processes this rule's output.
                field_plural_name = [ $field_plural_name:tt ]
                field_plural_name_mut = [ $field_plural_name_mut:tt ]
                field_comments = [ $(#[$field_comment:meta])* ]
            }
        )+ ]
    ) => {
        $crate::multi_vec::__private::paste! {
            // Table struct (e.g. `ScopeTable`)
            $(#[$table_attr])*
            #[derive(Default)]
            #[allow(dead_code, clippy::allow_attributes)]
            $vis struct $table_name {
                inner: $crate::multi_vec::__private::MultiVec<$key, $struct_name>,
            }

            // Item struct (e.g. `Scope`)
            $(#[$struct_attr])*
            #[allow(dead_code, clippy::allow_attributes)]
            $vis struct $struct_name {
                $(
                    $(#[$field_comment])*
                    pub $field_name: $field_ty,
                )+
            }

            // Item ref struct (e.g. `ScopeRef`)
            #[doc = concat!("References to one element of a [`", stringify!($table_name), "`].")]
            #[doc = ""]
            #[doc = concat!("Returned by [`", stringify!($table_name), "::get`].")]
            $(#[derive($debug)])?
            #[allow(dead_code, clippy::allow_attributes)]
            $vis struct [<$struct_name Ref>]<'v> {
                $(
                    $(#[$field_comment])*
                    pub $field_name: &'v $field_ty,
                )+
            }

            // Item mut ref struct (e.g. `ScopeMut`)
            #[doc = concat!("Mutable references to one element of a [`", stringify!($table_name), "`].")]
            #[doc = ""]
            #[doc = concat!("Returned by [`", stringify!($table_name), "::get_mut`].")]
            $(#[derive($debug)])?
            #[allow(dead_code, clippy::allow_attributes)]
            $vis struct [<$struct_name Mut>]<'v> {
                $(
                    $(#[$field_comment])*
                    pub $field_name: &'v mut $field_ty,
                )+
            }

            // Item slices struct (e.g. `ScopeSlices`)
            #[doc = concat!("Slices over each field array of a [`", stringify!($table_name), "`].")]
            #[doc = ""]
            #[doc = concat!("Returned by [`", stringify!($table_name), "::slices`].")]
            $(#[derive($debug)])?
            #[allow(dead_code, clippy::allow_attributes)]
            $vis struct [<$struct_name Slices>]<'v> {
                $(
                    pub $field_plural_name: &'v $crate::multi_vec::__private::IndexSlice<$key, [$field_ty]>,
                )+
            }

            // Item mutable slices struct (e.g. `ScopeSlicesMut`)
            #[doc = concat!("Mutable slices over each field array of a [`", stringify!($table_name), "`].")]
            #[doc = ""]
            #[doc = concat!("Returned by [`", stringify!($table_name), "::slices_mut`].")]
            $(#[derive($debug)])?
            #[allow(dead_code, clippy::allow_attributes)]
            $vis struct [<$struct_name SlicesMut>]<'v> {
                $(
                    pub $field_plural_name: &'v mut $crate::multi_vec::__private::IndexSlice<$key, [$field_ty]>,
                )+
            }

            // Iterator of refs (e.g. `ScopeTableIter`)
            #[doc = concat!("Iterator over a [`", stringify!($table_name), "`]'s elements, yielding a [`", stringify!($struct_name), "Ref`] for each.")]
            #[doc = ""]
            #[doc = concat!("Returned by [`", stringify!($table_name), "::iter`].")]
            #[must_use = "iterators are lazy and do nothing unless consumed"]
            #[allow(dead_code, clippy::allow_attributes)]
            $vis struct [<$table_name Iter>]<'v>(
                $crate::multi_vec::__private::Iter<'v, $struct_name>,
            );

            // Iterator of muts (e.g. `ScopeTableIterMut`)
            #[doc = concat!("Iterator over a [`", stringify!($table_name), "`]'s elements, yielding a [`", stringify!($struct_name), "Mut`] for each.")]
            #[doc = ""]
            #[doc = concat!("Returned by [`", stringify!($table_name), "::iter_mut`].")]
            #[must_use = "iterators are lazy and do nothing unless consumed"]
            #[allow(dead_code, clippy::allow_attributes)]
            $vis struct [<$table_name IterMut>]<'v>(
                $crate::multi_vec::__private::IterMut<'v, $struct_name>,
            );

            // Iterator of owned (e.g. `ScopeTableIntoIter`)
            #[doc = concat!("Iterator over a [`", stringify!($table_name), "`]'s elements, yielding each element as an owned [`", stringify!($struct_name), "`].")]
            #[doc = ""]
            #[doc = concat!("Returned by [`", stringify!($table_name), "`]'s [`IntoIterator`] impl.")]
            #[must_use = "iterators are lazy and do nothing unless consumed"]
            #[allow(dead_code, clippy::allow_attributes)]
            $vis struct [<$table_name IntoIter>](
                $crate::multi_vec::__private::IntoIter<$struct_name>,
            );

            // All the impls, and the items only they need (imports, `FIELD_COUNT`,
            // the compile-time assertions, and the type aliases), live in this anonymous
            // `const` block. It keeps them out of the invoking scope, while the impls
            // still apply to the types above as normal (and rustdoc still documents them).
            // Only the types themselves must be defined outside it, to be nameable by the user.
            #[allow(
                dead_code,
                unused_imports,
                forgetting_copy_types,
                private_interfaces,
                clippy::inline_always,
                clippy::macro_metavars_in_unsafe,
                clippy::undocumented_unsafe_blocks,
                clippy::allow_attributes
            )]
            const _: () = {
                use ::std::{alloc::Layout, fmt, iter::FusedIterator, mem, ptr::NonNull};

                use $crate::multi_vec::__private::{
                    CloneColumn, ColumnCloner, CopyColumn, IndexSlice, IntoIter, Iter, IterMut,
                    CloneFields, Fields, Shape, MultiVec, SrcAndDstPtrs, drop_column,
                    index_slice_from_raw_parts, index_slice_from_raw_parts_mut,
                };

                // Type aliases for the generated types, to simplify the code that follows.
                // These names can clash with user's types, but we accept that to make this
                // macro code more readable.
                type Ref<'v> = [<$struct_name Ref>]<'v>;
                type Mut<'v> = [<$struct_name Mut>]<'v>;
                type Slices<'v> = [<$struct_name Slices>]<'v>;
                type SlicesMut<'v> = [<$struct_name SlicesMut>]<'v>;

                type TableIter<'v> = [<$table_name Iter>]<'v>;
                type TableIterMut<'v> = [<$table_name IterMut>]<'v>;
                type TableIntoIter = [<$table_name IntoIter>];

                // Number of fields in the struct
                const FIELD_COUNT: usize = [$(stringify!($field_name)),+].len();

                // Verify that the item type named in the `table` declaration is the struct
                const _: fn($struct_name) -> $table_item_name = |item| item;

                impl $table_name {
                    #[doc = concat!("Maximum capacity of a `", stringify!($table_name), "`.")]
                    ///
                    /// Capacity is limited by the index type's range, and the maximum
                    /// allocation size (`isize::MAX` bytes).
                    pub const MAX_CAPACITY: usize = MultiVec::<$key, $struct_name>::MAX_CAPACITY;

                    #[doc = concat!("Create a new empty `", stringify!($table_name), "`. Does not allocate.")]
                    #[inline(always)]
                    pub const fn new() -> Self {
                        Self { inner: MultiVec::new() }
                    }

                    #[doc = concat!("Create a new `", stringify!($table_name), "` with capacity for `capacity` elements.")]
                    ///
                    /// Does not allocate if `capacity == 0`.
                    ///
                    /// # Panics
                    ///
                    /// Panics if `capacity` exceeds [`MAX_CAPACITY`].
                    ///
                    /// [`MAX_CAPACITY`]: Self::MAX_CAPACITY
                    #[inline(always)]
                    pub fn with_capacity(capacity: usize) -> Self {
                        Self { inner: MultiVec::with_capacity(capacity) }
                    }

                    /// Returns the number of elements.
                    #[inline(always)]
                    pub fn len(&self) -> usize {
                        self.inner.len()
                    }

                    /// Returns `true` if there are no elements.
                    #[inline(always)]
                    pub fn is_empty(&self) -> bool {
                        self.inner.is_empty()
                    }

                    #[doc = concat!("Push a [`", stringify!($struct_name), "`] to this [`", stringify!($table_name), "`].")]
                    #[doc = concat!("Returns the [`", stringify!($key), "`] of the new element.")]
                    ///
                    /// # Panics
                    ///
                    /// Panics if already full to [`MAX_CAPACITY`].
                    ///
                    /// [`MAX_CAPACITY`]: Self::MAX_CAPACITY
                    #[inline(always)]
                    pub fn push(&mut self, value: $struct_name) -> $key {
                        self.inner.push(value)
                    }

                    /// Reserve capacity for at least `additional` more elements.
                    ///
                    /// # Panics
                    ///
                    /// Panics if the required capacity exceeds [`MAX_CAPACITY`].
                    ///
                    /// [`MAX_CAPACITY`]: Self::MAX_CAPACITY
                    #[inline(always)]
                    pub fn reserve(&mut self, additional: usize) {
                        self.inner.reserve(additional);
                    }

                    #[doc = concat!("Get references to the element at `", stringify!($id), "`.")]
                    ///
                    /// # Panics
                    ///
                    #[doc = concat!("Panics if `", stringify!($id), "` is out of bounds.")]
                    #[inline(always)]
                    pub fn get(&self, $id: $key) -> Ref<'_> {
                        self.inner.get($id)
                    }

                    #[doc = concat!("Get mutable references to the element at `", stringify!($id), "`.")]
                    ///
                    /// # Panics
                    ///
                    #[doc = concat!("Panics if `", stringify!($id), "` is out of bounds.")]
                    #[inline(always)]
                    pub fn get_mut(&mut self, $id: $key) -> Mut<'_> {
                        self.inner.get_mut($id)
                    }

                    #[doc = concat!("Get references to the element at `", stringify!($id), "`,")]
                    #[doc = concat!("without checking that `", stringify!($id), "` is in bounds.")]
                    ///
                    /// # SAFETY
                    ///
                    #[doc = concat!("`", stringify!($id), "` must be in bounds - less than [`len`](Self::len).")]
                    #[inline(always)]
                    pub unsafe fn get_unchecked(&self, $id: $key) -> Ref<'_> {
                        // SAFETY: Caller guarantees `$id` is in bounds`
                        unsafe { self.inner.get_unchecked($id) }
                    }

                    #[doc = concat!("Get mutable references to the element at `", stringify!($id), "`,")]
                    #[doc = concat!("without checking that `", stringify!($id), "` is in bounds.")]
                    ///
                    /// # SAFETY
                    ///
                    #[doc = concat!("`", stringify!($id), "` must be in bounds - less than [`len`](Self::len).")]
                    #[inline(always)]
                    pub unsafe fn get_unchecked_mut(&mut self, $id: $key) -> Mut<'_> {
                        // SAFETY: Caller guarantees `$id` is in bounds
                        unsafe { self.inner.get_unchecked_mut($id) }
                    }

                    /// Get slices over all field arrays.
                    #[inline(always)]
                    pub fn slices(&self) -> Slices<'_> {
                        self.inner.slices()
                    }

                    /// Get mutable slices over all field arrays.
                    #[inline(always)]
                    pub fn slices_mut(&mut self) -> SlicesMut<'_> {
                        self.inner.slices_mut()
                    }

                    /// Iterate over all valid indices.
                    ///
                    /// The returned iterator does not borrow `self` (the `use<...>` clause omits
                    /// the `&self` lifetime, opting out of edition 2024's capture-everything default).
                    /// It snapshots `len`, so the table can be mutated while the iterator lives.
                    /// IDs of elements pushed after the `iter_ids` call are not included.
                    #[inline(always)]
                    pub fn iter_ids(&self) -> impl ExactSizeIterator<Item = $key> + FusedIterator + use<> {
                        self.inner.iter_ids()
                    }

                    #[doc = concat!("Iterate over all elements, yielding a [`", stringify!($struct_name), "Ref`] (references to every field) for each.")]
                    #[inline(always)]
                    pub fn iter(&self) -> TableIter<'_> {
                        [<$table_name Iter>](self.inner.iter())
                    }

                    #[doc = concat!("Iterate over all elements, yielding a [`", stringify!($struct_name), "Mut`] (mutable references to every field) for each.")]
                    #[inline(always)]
                    pub fn iter_mut(&mut self) -> TableIterMut<'_> {
                        [<$table_name IterMut>](self.inner.iter_mut())
                    }

                    #[doc = concat!("Iterate over all elements, yielding each element's [`", stringify!($key), "`] and a [`", stringify!($struct_name), "Ref`].")]
                    #[inline(always)]
                    pub fn iter_enumerated(&self) -> impl ExactSizeIterator<Item = ($key, Ref<'_>)> + FusedIterator {
                        self.inner.iter_enumerated()
                    }

                    #[doc = concat!("Iterate over all elements, yielding each element's [`", stringify!($key), "`] and a [`", stringify!($struct_name), "Mut`].")]
                    #[inline(always)]
                    pub fn iter_mut_enumerated(&mut self) -> impl ExactSizeIterator<Item = ($key, Mut<'_>)> + FusedIterator {
                        self.inner.iter_mut_enumerated()
                    }

                    #[doc = concat!("Consume the table, yielding each element's [`", stringify!($key), "`] and the element as an owned [`", stringify!($struct_name), "`].")]
                    #[inline(always)]
                    pub fn into_iter_enumerated(self) -> impl ExactSizeIterator<Item = ($key, $struct_name)> + FusedIterator {
                        self.inner.into_iter_enumerated()
                    }

                    // Per-field accessor methods, e.g. `.parent_id(id)` / `.parent_id_mut(id)`.
                    $(
                        #[doc = concat!("Get reference to the `", stringify!($field_name), "` field of the element at `", stringify!($id), "`.")]
                        ///
                        /// # Panics
                        ///
                        #[doc = concat!("Panics if `", stringify!($id), "` is out of bounds.")]
                        #[inline(always)]
                        pub fn $field_name(&self, $id: $key) -> &$field_ty {
                            self.get($id).$field_name
                        }

                        #[doc = concat!("Get mutable reference to the `", stringify!($field_name), "` field of the element at `", stringify!($id), "`.")]
                        ///
                        /// # Panics
                        ///
                        #[doc = concat!("Panics if `", stringify!($id), "` is out of bounds.")]
                        #[inline(always)]
                        pub fn [<$field_name _mut>](&mut self, $id: $key) -> &mut $field_ty {
                            self.get_mut($id).$field_name
                        }

                        #[doc = concat!("Get slice of `", stringify!($field_name), "` fields of all elements.")]
                        #[inline(always)]
                        pub fn $field_plural_name(&self) -> &IndexSlice<$key, [$field_ty]> {
                            self.slices().$field_plural_name
                        }

                        #[doc = concat!("Get mutable slice of `", stringify!($field_name), "` fields of all elements.")]
                        #[inline(always)]
                        pub fn $field_plural_name_mut(&mut self) -> &mut IndexSlice<$key, [$field_ty]> {
                            self.slices_mut().$field_plural_name
                        }
                    )+
                }

                impl IntoIterator for $table_name {
                    type Item = $struct_name;
                    type IntoIter = TableIntoIter;

                    #[doc = concat!("Consume the table, yielding each element as an owned [`", stringify!($struct_name), "`], reassembled from its stored fields.")]
                    #[inline(always)]
                    fn into_iter(self) -> TableIntoIter {
                        [<$table_name IntoIter>](self.inner.into_iter())
                    }
                }

                impl<'v> IntoIterator for &'v $table_name {
                    type Item = Ref<'v>;
                    type IntoIter = TableIter<'v>;

                    #[inline(always)]
                    fn into_iter(self) -> TableIter<'v> {
                        self.iter()
                    }
                }

                impl<'v> IntoIterator for &'v mut $table_name {
                    type Item = Mut<'v>;
                    type IntoIter = TableIterMut<'v>;

                    #[inline(always)]
                    fn into_iter(self) -> TableIterMut<'v> {
                        self.iter_mut()
                    }
                }

                // Implement `Debug` on the table type if `#[derive(Debug)]` is present.
                // Prints as a map from ID to element e.g. `{ 0: ScopeRef { ... }, 1: ... }`.
                $(
                    // `#[derive(Debug)]` on the `table` requires the struct to be `Debug` too
                    // (like `Clone` - see "Debug" in the macro docs). Verify it at check time.
                    // The impl below will also fail to compile if the struct is not `Debug`,
                    // but with a poor error message. This fails first, naming the offending type.
                    const _: fn(&$struct_name, &mut fmt::Formatter<'_>) -> fmt::Result =
                        <$struct_name as fmt::$debug>::fmt;

                    impl fmt::$debug for $table_name {
                        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                            f.debug_map().entries(self.iter_enumerated()).finish()
                        }
                    }
                )?

                const SHAPE: Shape<[usize; FIELD_COUNT]> = Shape::new([ $( Layout::new::<$field_ty>() ),+ ]);

                // Implement `Fields` on the struct type (e.g. `Scope`).
                // These methods are a thin layer of type casting.
                //
                // They all either:
                // 1. Convert `NonNull<u8>` pointers to typed references, or
                // 2. Call typed methods.
                unsafe impl Fields for $struct_name {
                    type Ref<'v> = Ref<'v> where Self: 'v;
                    type Mut<'v> = Mut<'v> where Self: 'v;
                    type Slices<'v> = Slices<'v> where Self: 'v;
                    type SlicesMut<'v> = SlicesMut<'v> where Self: 'v;

                    type Array<T: Copy> = [T; FIELD_COUNT];

                    const SHAPE: Shape<[usize; FIELD_COUNT]> = SHAPE;

                    #[inline]
                    unsafe fn write(self, ptrs: [NonNull<u8>; FIELD_COUNT]) {
                        let [ $($field_name),+ ] = ptrs;
                        unsafe {
                            $(
                                $field_name.cast::<$field_ty>().write(self.$field_name);
                            )+
                        }
                    }

                    #[inline]
                    unsafe fn read(ptrs: [NonNull<u8>; FIELD_COUNT]) -> Self {
                        let [ $($field_name),+ ] = ptrs;
                        unsafe {
                            Self {
                                $(
                                    $field_name: $field_name.cast::<$field_ty>().read(),
                                )+
                            }
                        }
                    }

                    #[inline]
                    unsafe fn create_ref<'v>(ptrs: [NonNull<u8>; FIELD_COUNT]) -> Ref<'v> {
                        let [ $($field_name),+ ] = ptrs;
                        unsafe {
                            Ref {
                                $(
                                    $field_name: $field_name.cast::<$field_ty>().as_ref(),
                                )+
                            }
                        }
                    }

                    #[inline]
                    unsafe fn create_mut<'v>(ptrs: [NonNull<u8>; FIELD_COUNT]) -> Mut<'v> {
                        let [ $($field_name),+ ] = ptrs;
                        unsafe {
                            Mut {
                                $(
                                    $field_name: $field_name.cast::<$field_ty>().as_mut(),
                                )+
                            }
                        }
                    }

                    #[inline]
                    unsafe fn slices<'v>(ptrs: [NonNull<u8>; FIELD_COUNT], len: usize) -> Slices<'v> {
                        let [ $($field_name),+ ] = ptrs;
                        unsafe {
                            Slices {
                                $(
                                    $field_plural_name: index_slice_from_raw_parts($field_name, len),
                                )+
                            }
                        }
                    }

                    #[inline]
                    unsafe fn slices_mut<'v>(ptrs: [NonNull<u8>; FIELD_COUNT], len: usize) -> SlicesMut<'v> {
                        let [ $($field_name),+ ] = ptrs;
                        unsafe {
                            SlicesMut {
                                $(
                                    $field_plural_name: index_slice_from_raw_parts_mut($field_name, len),
                                )+
                            }
                        }
                    }

                    #[inline]
                    unsafe fn drop_columns(ptrs: [NonNull<u8>; FIELD_COUNT], len: usize) {
                        let [ $($field_name),+ ] = ptrs;
                        unsafe {
                            $(
                                drop_column::<$field_ty>($field_name, len);
                            )+
                        }
                    }
                }

                // Implement `CloneFields` on the struct type (e.g. `Scope`).
                // `@generate_clone` expands to nothing when `clone` is empty.
                $crate::multi_vec! {
                    @generate_clone
                    clone = $clone
                    struct_name = [ $struct_name ]
                    fields = [ $(
                        {
                            field_name = [ $field_name ]
                            field_ty = [ $field_ty ]
                        }
                    )+ ]
                }

                // Iterator of refs (e.g. `ScopeTableIter`)
                impl<'v> Iterator for TableIter<'v> {
                    type Item = Ref<'v>;

                    #[inline(always)]
                    fn next(&mut self) -> Option<Ref<'v>> {
                        self.0.next()
                    }

                    #[inline(always)]
                    fn size_hint(&self) -> (usize, Option<usize>) {
                        self.0.size_hint()
                    }
                }

                impl ExactSizeIterator for TableIter<'_> {}

                impl FusedIterator for TableIter<'_> {}

                // Iterator of mut refs (e.g. `ScopeTableIterMut`)
                impl<'v> Iterator for TableIterMut<'v> {
                    type Item = Mut<'v>;

                    #[inline(always)]
                    fn next(&mut self) -> Option<Mut<'v>> {
                        self.0.next()
                    }

                    #[inline(always)]
                    fn size_hint(&self) -> (usize, Option<usize>) {
                        self.0.size_hint()
                    }
                }

                impl ExactSizeIterator for TableIterMut<'_> {}

                impl FusedIterator for TableIterMut<'_> {}

                // Iterator of owned items (e.g. `ScopeTableIntoIter`)
                impl Iterator for TableIntoIter {
                    type Item = $struct_name;

                    #[inline(always)]
                    fn next(&mut self) -> Option<$struct_name> {
                        self.0.next()
                    }

                    #[inline(always)]
                    fn size_hint(&self) -> (usize, Option<usize>) {
                        self.0.size_hint()
                    }
                }

                impl ExactSizeIterator for TableIntoIter {}

                impl FusedIterator for TableIntoIter {}
            };
        }
    };

    // Generate the clone machinery - the `CloneFields` impl, and a per-field `Clone`
    // assertion - for a table declared with `#[derive(Clone)]`.
    //
    // Tables without the derive get no clone machinery at all.
    // `MultiVec` is `Clone` only where its fields struct implements `CloneFields`,
    // so the `#[derive(Clone)]` passed through to the table type resolves exactly
    // when this impl exists.
    //
    // Invoked by `@generate`, unconditionally, from inside the generated `const` block,
    // so the block's imports and `FIELD_COUNT` are in scope.
    // The rule below expands to nothing when the `clone` slot is empty.
    // The choice must be made by rule matching, in a separate rule -
    // `@generate` could only make it with a `$( ... )?` group gated on the `clone` slot
    // and this impl's per-field code cannot live inside one - a metavariable
    // cannot be used under more repetition layers than it was bound with.
    (
        @generate_clone
        clone = [ Clone ]
        struct_name = [ $struct_name:ident ]
        fields = [ $(
            {
                field_name = [ $field_name:ident ]
                field_ty = [ $field_ty:ty ]
            }
        )+ ]
    ) => {
        // `#[derive(Clone)]` on the `table` requires every field type to be `Clone`.
        // The `clone_column` calls below would also fail to resolve for a non-`Clone` field,
        // but with a poor error message. These fail first, naming the offending type.
        $(
            const _: fn(&$field_ty) -> $field_ty = <$field_ty as Clone>::clone;
        )+

        // Implement `CloneFields` on the struct type (e.g. `Scope`)
        unsafe impl CloneFields for $struct_name {
            #[inline]
            unsafe fn clone_columns(src_and_dst_ptrs: [SrcAndDstPtrs; FIELD_COUNT], len: usize) {
                let [ $($field_name),+ ] = src_and_dst_ptrs;

                // Each `Clone` (non-`Copy`) column's `clone_column` returns a `ColumnDropGuard`
                // (`Copy` columns return `()`). The guards are held in a tuple until all columns
                // are cloned, then forgotten - so if a column's `clone` panics, unwinding drops
                // all values cloned so far, in this column (by `clone_column`'s internal guard)
                // and in earlier columns (by the already-built guards of the partially-evaluated tuple).
                let drop_guards = unsafe {
                    (
                        $(
                            (&&ColumnCloner::<$field_ty>::NEW).clone_column($field_name, len),
                        )+
                    )
                };
                mem::forget(drop_guards);
            }
        }
    };

    // No `#[derive(Clone)]` on the `table` - no clone machinery.
    (
        @generate_clone
        clone = []
        struct_name = $_struct_name:tt
        fields = $_fields:tt
    ) => {};
}
pub use multi_vec;
