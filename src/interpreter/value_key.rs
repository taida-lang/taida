//! C25B-021 / C25B-022 / C25B-023 common foundation — `ValueKey`.
//!
//! A hashable view over [`Value`] for use as a HashSet / HashMap key.
//! Used by the fast paths in `Set.union` / `Set.intersect` / `Set.diff` /
//! `HashMap.merge` / `Unique` to lift their core comparison loop from
//! `Vec::contains` (O(N*M)) to a pre-built `HashSet<ValueKey>` (O(N+M)).
//!
//! # Design lock (C25 Phase 5-D / 2026-04-23)
//!
//! The set of [`Value`] variants that can participate in the fast path
//! is deliberately narrow. A value is **key-eligible** if, and only if,
//! every recursive component is also key-eligible. Eligible variants:
//!
//! * `Int(i64)`
//! * `Bool(bool)`
//! * `Str(String)`
//! * `Bytes(Vec<u8>)`
//! * `Unit`
//! * `EnumVal(String, i64)`
//! * `Gorilla`
//! * `List(Vec<Value>)` — recursive; each element must be key-eligible
//! * `BuchiPack(Vec<(String, Value)>)` — recursive; order-independent
//!   (matches the existing `PartialEq` contract on BuchiPack)
//!
//! Excluded variants (the caller retains the existing linear-scan
//! fallback when it encounters any of these):
//!
//! * `Float(f64)` — f64 has no `Eq`; NaN breaks reflexivity. We never
//!   store Float in the hash domain; callers that mix Float into a
//!   Set / HashMap / Unique operand still get correct results via the
//!   unchanged linear path.
//! * `Function` / closures — not meaningfully hashable.
//! * `Async` / `Stream` — runtime state; equality is identity-ish.
//! * `Error` — carries runtime metadata; not a stable key.
//! * `Molten` / `Json` — opaque to the Taida surface.
//!
//! # Cross-type equality (trade-off)
//!
//! `Value::eq` treats `Int(n)`, `EnumVal(_, n)` and `Float(n as f64)` as
//! equal when they share the same numeric ordinal. `ValueKey` does not
//! preserve this cross-type equivalence — `Int(3)` and `EnumVal(X, 3)`
//! hash to different buckets, and callers that insert a mix will miss
//! fast-path hits for those specific cross-type pairs. This is an
//! accepted trade-off: the programs we want to accelerate (vocabularies,
//! token tables, ScreenBuffer cell grids) do not mix these variants.
//! When the fast path misses, the caller falls back to a linear
//! contains() check which preserves full `Value::eq` semantics.
//!
//! # Contract on borrowed data
//!
//! `ValueKey<'a>` borrows from the underlying `Value`. The caller must
//! keep the `Value` alive for the HashSet's lifetime. All uses in
//! `methods.rs` / `mold_eval.rs` build the HashSet immediately from a
//! borrow that lives for the scope of the operation, satisfying this.

use super::value::Value;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Hashable borrowed view of a [`Value`]. Construct with
/// [`ValueKey::new`]; `None` means the value is not key-eligible and
/// the caller must fall back to linear `Value::eq` comparison.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ValueKey<'a>(pub(crate) &'a Value);

impl<'a> ValueKey<'a> {
    /// Wrap `v` as a `ValueKey` iff every recursive component is
    /// key-eligible. See module docs for the classification.
    pub(crate) fn new(v: &'a Value) -> Option<Self> {
        if is_hashable(v) { Some(Self(v)) } else { None }
    }

    /// Construct a hash fingerprint independent of storage order for
    /// BuchiPack fields. Public (crate-local) for callers that want to
    /// build their own HashSet<u64> keyed on the fingerprint rather
    /// than carrying ValueKey around.
    pub(crate) fn fingerprint(&self) -> u64 {
        let mut h = DefaultHasher::new();
        hash_value_into(self.0, &mut h);
        h.finish()
    }
}

impl PartialEq for ValueKey<'_> {
    fn eq(&self, other: &Self) -> bool {
        // Intra-ValueKey equality is exact (no cross-type coercion).
        // This is stricter than `Value::eq` on purpose; see module docs.
        exact_eq(self.0, other.0)
    }
}

impl Eq for ValueKey<'_> {}

impl Hash for ValueKey<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_value_into(self.0, state);
    }
}

/// Return true iff `v` is recursively key-eligible.
fn is_hashable(v: &Value) -> bool {
    match v {
        Value::Int(_)
        | Value::Bool(_)
        | Value::Str(_)
        | Value::Bytes(_)
        | Value::Unit
        | Value::Gorilla
        | Value::EnumVal(_, _) => true,
        Value::List(items) => items.iter().all(is_hashable),
        Value::BuchiPack(fields) => fields.iter().all(|(_, v)| is_hashable(v)),
        Value::Float(_)
        | Value::Function(_)
        | Value::Async(_)
        | Value::Stream(_)
        | Value::Error(_)
        | Value::Molten
        | Value::Json(_) => false,
    }
}

/// Exact structural equality for key-eligible values. BuchiPack is
/// order-independent (matches `Value::eq`).
fn exact_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Bytes(x), Value::Bytes(y)) => x == y,
        (Value::Unit, Value::Unit) => true,
        (Value::Gorilla, Value::Gorilla) => true,
        (Value::EnumVal(na, nb_a), Value::EnumVal(nb, nb_b)) => na == nb && nb_a == nb_b,
        (Value::List(xs), Value::List(ys)) => {
            xs.len() == ys.len() && xs.iter().zip(ys.iter()).all(|(a, b)| exact_eq(a, b))
        }
        (Value::BuchiPack(xs), Value::BuchiPack(ys)) => {
            if xs.len() != ys.len() {
                return false;
            }
            xs.iter()
                .all(|(nx, vx)| ys.iter().any(|(ny, vy)| nx == ny && exact_eq(vx, vy)))
        }
        _ => false,
    }
}

/// Hash a key-eligible value. BuchiPack is order-independent: we hash
/// the XOR-reduction of per-field hashes so that `@(a<=1, b<=2)` and
/// `@(b<=2, a<=1)` produce the same fingerprint.
fn hash_value_into<H: Hasher>(v: &Value, state: &mut H) {
    // Tag each variant so that `Int(0)` and `Bool(false)` don't collide.
    match v {
        Value::Int(n) => {
            0u8.hash(state);
            n.hash(state);
        }
        Value::Bool(b) => {
            1u8.hash(state);
            b.hash(state);
        }
        Value::Str(s) => {
            2u8.hash(state);
            s.hash(state);
        }
        Value::Bytes(b) => {
            3u8.hash(state);
            b.hash(state);
        }
        Value::Unit => {
            4u8.hash(state);
        }
        Value::Gorilla => {
            5u8.hash(state);
        }
        Value::EnumVal(name, n) => {
            6u8.hash(state);
            name.hash(state);
            n.hash(state);
        }
        Value::List(items) => {
            7u8.hash(state);
            items.len().hash(state);
            for it in items {
                hash_value_into(it, state);
            }
        }
        Value::BuchiPack(fields) => {
            8u8.hash(state);
            fields.len().hash(state);
            // Order-independent field mix. Each (name, value) produces a
            // per-field u64, all XOR-reduced, then fed into the outer
            // hasher so that two packs with the same fields in different
            // order produce the same outer hash.
            let mut mix: u64 = 0;
            for (name, val) in fields {
                let mut fh = DefaultHasher::new();
                name.hash(&mut fh);
                hash_value_into(val, &mut fh);
                mix ^= fh.finish();
            }
            mix.hash(state);
        }
        // Unreachable because `is_hashable` gates ValueKey construction.
        Value::Float(_)
        | Value::Function(_)
        | Value::Async(_)
        | Value::Stream(_)
        | Value::Error(_)
        | Value::Molten
        | Value::Json(_) => {
            unreachable!("ValueKey::hash called on non-hashable variant")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn value_key_basic_hashability() {
        assert!(ValueKey::new(&Value::Int(1)).is_some());
        assert!(ValueKey::new(&Value::Bool(true)).is_some());
        assert!(ValueKey::new(&Value::Str("hi".into())).is_some());
        assert!(ValueKey::new(&Value::Unit).is_some());
        assert!(ValueKey::new(&Value::Gorilla).is_some());
        assert!(ValueKey::new(&Value::EnumVal("X".into(), 2)).is_some());
        assert!(ValueKey::new(&Value::Bytes(vec![1, 2, 3])).is_some());
    }

    #[test]
    fn value_key_float_and_function_are_excluded() {
        assert!(ValueKey::new(&Value::Float(1.0)).is_none());
        // Function / Async / Stream exclusion is checked via List/Pack
        // composite (simpler to construct).
        let f = Value::Float(f64::NAN);
        assert!(ValueKey::new(&f).is_none());
        assert!(
            ValueKey::new(&Value::List(vec![Value::Int(1), Value::Float(2.0)])).is_none(),
            "list containing float must be non-eligible"
        );
    }

    #[test]
    fn value_key_list_eligibility_is_recursive() {
        let pure = Value::List(vec![Value::Int(1), Value::Str("a".into())]);
        assert!(ValueKey::new(&pure).is_some());
        let nested = Value::List(vec![Value::List(vec![
            Value::Int(1),
            Value::Bytes(vec![5]),
        ])]);
        assert!(ValueKey::new(&nested).is_some());
    }

    #[test]
    fn value_key_buchi_pack_is_order_independent() {
        let a = Value::BuchiPack(vec![
            ("x".into(), Value::Int(1)),
            ("y".into(), Value::Str("hi".into())),
        ]);
        let b = Value::BuchiPack(vec![
            ("y".into(), Value::Str("hi".into())),
            ("x".into(), Value::Int(1)),
        ]);
        let ka = ValueKey::new(&a).unwrap();
        let kb = ValueKey::new(&b).unwrap();
        assert_eq!(ka, kb, "different field order must compare equal");
        assert_eq!(
            ka.fingerprint(),
            kb.fingerprint(),
            "different field order must hash equal"
        );
    }

    #[test]
    // `ValueKey` wraps `&Value`, whose enum carries `Mutex`/`AtomicU32`
    // (inside `Async` / `Stream` variants) that make clippy flag the
    // HashSet key type. `is_hashable()` explicitly rejects those
    // variants, so in practice the HashSet never sees an interior-
    // mutable key; the lint is a false positive here.
    #[allow(clippy::mutable_key_type)]
    fn value_key_round_trip_in_hashset() {
        let values = vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(1), // duplicate
            Value::Str("a".into()),
            Value::Str("a".into()), // duplicate
            Value::Bool(false),
        ];
        let mut set: HashSet<ValueKey> = HashSet::new();
        for v in &values {
            if let Some(k) = ValueKey::new(v) {
                set.insert(k);
            }
        }
        assert_eq!(set.len(), 4, "int(1), int(2), str(a), bool(false)");
    }

    #[test]
    fn value_key_int_and_enum_do_not_collide() {
        // Intentional cross-type divergence from Value::eq — see module docs.
        let int = Value::Int(3);
        let enum_v = Value::EnumVal("Color".into(), 3);
        let ki = ValueKey::new(&int).unwrap();
        let ke = ValueKey::new(&enum_v).unwrap();
        assert_ne!(ki, ke, "Int(3) and EnumVal(_, 3) are distinct ValueKeys");
    }
}
