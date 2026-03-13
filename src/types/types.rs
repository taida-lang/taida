/// Taida Lang type system.
///
/// All types have default values — null/undefined does not exist.
/// Structural subtyping: a value with extra fields is compatible
/// with a type that expects fewer fields (width subtyping).
use std::collections::HashMap;
use std::fmt;

type MoldDefFields = (Vec<String>, Vec<(String, Type)>);

/// A Taida type.
#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    /// Integer type
    Int,
    /// Floating-point type
    Float,
    /// General number type (Int or Float)
    Num,
    /// String type
    Str,
    /// Bytes type (immutable byte sequence)
    Bytes,
    /// Boolean type
    Bool,
    /// Buchi pack type (named fields)
    BuchiPack(Vec<(String, Type)>),
    /// List type
    List(Box<Type>),
    /// Function type (params -> return)
    Function(Vec<Type>, Box<Type>),
    /// Named user-defined type
    Named(String),
    /// Generic / Mold type instantiation: e.g., Optional[Int]
    Generic(String, Vec<Type>),
    /// Error type (inherits from base Error)
    Error(String),
    /// Unit type (empty buchi pack)
    Unit,
    /// Unknown / not yet inferred
    Unknown,
    /// Any type — used internally for type inference, never user-visible
    Any,
    /// JSON type — opaque external data primitive
    Json,
    /// Molten type — opaque primitive for external (JS) interop data
    Molten,
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Int => write!(f, "Int"),
            Type::Float => write!(f, "Float"),
            Type::Num => write!(f, "Num"),
            Type::Str => write!(f, "Str"),
            Type::Bytes => write!(f, "Bytes"),
            Type::Bool => write!(f, "Bool"),
            Type::BuchiPack(fields) => {
                write!(f, "@(")?;
                for (i, (name, ty)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", name, ty)?;
                }
                write!(f, ")")
            }
            Type::List(inner) => write!(f, "@[{}]", inner),
            Type::Function(params, ret) => {
                write!(f, "(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ") => {}", ret)
            }
            Type::Named(name) => write!(f, "{}", name),
            Type::Generic(name, args) => {
                write!(f, "{}[", name)?;
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", a)?;
                }
                write!(f, "]")
            }
            Type::Error(name) => write!(f, "{}", name),
            Type::Unit => write!(f, "@()"),
            Type::Unknown => write!(f, "?"),
            Type::Any => write!(f, "Any"),
            Type::Json => write!(f, "JSON"),
            Type::Molten => write!(f, "Molten"),
        }
    }
}

impl Type {
    /// Returns the default value for a type as a runtime Value.
    /// Every Taida type must have a default value (no null/undefined).
    pub fn default_value_description(&self) -> &'static str {
        match self {
            Type::Int => "0",
            Type::Float => "0.0",
            Type::Num => "0",
            Type::Str => "\"\"",
            Type::Bytes => "Bytes[]",
            Type::Bool => "false",
            Type::BuchiPack(_) => "@(...defaults)",
            Type::List(_) => "@[]",
            Type::Function(_, _) => "<noop>",
            Type::Named(_) => "<default>",
            Type::Generic(_, _) => "<default>",
            Type::Error(_) => "Error(type: \"\", message: \"\")",
            Type::Unit => "@()",
            Type::Unknown => "<unknown>",
            Type::Any => "<any>",
            Type::Json => "{}",
            Type::Molten => "Molten",
        }
    }

    /// Check if this type is a numeric type.
    pub fn is_numeric(&self) -> bool {
        matches!(self, Type::Int | Type::Float | Type::Num)
    }

    /// Check structural subtype compatibility: `self` is a subtype of `expected`.
    /// Width subtyping: a buchi pack with extra fields is a subtype of one with fewer.
    pub fn is_subtype_of(&self, expected: &Type) -> bool {
        if self == expected {
            return true;
        }

        match (self, expected) {
            // Any accepts everything
            (_, Type::Any) | (_, Type::Unknown) => true,
            (Type::Any, _) | (Type::Unknown, _) => true,

            // Int is a subtype of Num, Float is a subtype of Num
            (Type::Int, Type::Num) | (Type::Float, Type::Num) => true,
            (Type::Int, Type::Float) => true, // Int widened to Float

            // Structural subtyping for buchi packs
            (Type::BuchiPack(self_fields), Type::BuchiPack(expected_fields)) => {
                // All fields in expected must exist in self with compatible types
                expected_fields.iter().all(|(exp_name, exp_type)| {
                    self_fields.iter().any(|(self_name, self_type)| {
                        self_name == exp_name && self_type.is_subtype_of(exp_type)
                    })
                })
            }

            // List covariance
            (Type::List(a), Type::List(b)) => a.is_subtype_of(b),

            // Error subtyping: specific error is subtype of Error
            (Type::Error(_), Type::Error(name)) if name == "Error" => true,
            (Type::Error(_), Type::Named(name)) if name == "Error" => true,

            // Named type could match by name
            (Type::Named(a), Type::Named(b)) => a == b,

            // Generic types
            (Type::Generic(a_name, a_args), Type::Generic(b_name, b_args)) => {
                a_name == b_name
                    && a_args.len() == b_args.len()
                    && a_args
                        .iter()
                        .zip(b_args.iter())
                        .all(|(a, b)| a.is_subtype_of(b))
            }

            _ => false,
        }
    }
}

/// Type definition registry — stores all user-defined types.
#[derive(Debug, Clone, Default)]
pub struct TypeRegistry {
    /// Named type definitions: name -> fields
    pub type_defs: HashMap<String, Vec<(String, Type)>>,
    /// Mold type definitions: name -> (type_params, fields)
    pub mold_defs: HashMap<String, MoldDefFields>,
    /// Inheritance relationships: child -> parent
    pub inheritance: HashMap<String, String>,
    /// Error type definitions (inherit from Error)
    pub error_types: HashMap<String, Vec<(String, Type)>>,
}

impl TypeRegistry {
    pub fn new() -> Self {
        let mut registry = Self::default();
        // Register built-in Error base type
        registry.type_defs.insert(
            "Error".to_string(),
            vec![
                ("type".to_string(), Type::Str),
                ("message".to_string(), Type::Str),
            ],
        );
        registry
    }

    /// Register a type definition.
    pub fn register_type(&mut self, name: &str, fields: Vec<(String, Type)>) {
        self.type_defs.insert(name.to_string(), fields);
    }

    /// Register a mold type definition.
    pub fn register_mold(
        &mut self,
        name: &str,
        type_params: Vec<String>,
        fields: Vec<(String, Type)>,
    ) {
        self.mold_defs
            .insert(name.to_string(), (type_params, fields));
    }

    /// Register an inheritance relationship.
    pub fn register_inheritance(
        &mut self,
        parent: &str,
        child: &str,
        extra_fields: Vec<(String, Type)>,
    ) {
        self.inheritance
            .insert(child.to_string(), parent.to_string());

        // Child type gets parent fields + its own fields
        let mut fields = self.get_type_fields(parent).unwrap_or_default();
        fields.extend(extra_fields);
        self.type_defs.insert(child.to_string(), fields);
    }

    /// Register an error type (inherits from Error).
    pub fn register_error_type(&mut self, name: &str, extra_fields: Vec<(String, Type)>) {
        let mut fields = vec![
            ("type".to_string(), Type::Str),
            ("message".to_string(), Type::Str),
        ];
        fields.extend(extra_fields.clone());
        self.type_defs.insert(name.to_string(), fields);
        self.error_types.insert(name.to_string(), extra_fields);
        self.inheritance
            .insert(name.to_string(), "Error".to_string());
    }

    /// Get the fields of a type definition.
    pub fn get_type_fields(&self, name: &str) -> Option<Vec<(String, Type)>> {
        self.type_defs.get(name).cloned()
    }

    /// Check if a type is an error type (inherits from Error).
    pub fn is_error_type(&self, name: &str) -> bool {
        name == "Error" || self.error_types.contains_key(name)
    }

    /// Check structural subtype compatibility with registry context.
    /// Resolves Named types to their fields for structural comparison.
    pub fn is_subtype_of(&self, actual: &Type, expected: &Type) -> bool {
        if actual == expected {
            return true;
        }
        // Delegate to the basic check first
        if actual.is_subtype_of(expected) {
            return true;
        }
        match (actual, expected) {
            // Named vs Named: check inheritance chain, then structural fields
            (Type::Named(a), Type::Named(b)) => {
                // Check inheritance chain: a inherits from b?
                let mut current = a.clone();
                while let Some(parent) = self.inheritance.get(&current) {
                    if parent == b {
                        return true;
                    }
                    current = parent.clone();
                }
                // Structural check: a's fields are a superset of b's fields
                if let (Some(a_fields), Some(b_fields)) =
                    (self.get_type_fields(a), self.get_type_fields(b))
                {
                    b_fields.iter().all(|(exp_name, exp_type)| {
                        a_fields.iter().any(|(self_name, self_type)| {
                            self_name == exp_name && self_type.is_subtype_of(exp_type)
                        })
                    })
                } else {
                    false
                }
            }
            // Named vs BuchiPack: resolve Named and check structurally
            (Type::Named(name), Type::BuchiPack(expected_fields)) => {
                if let Some(actual_fields) = self.get_type_fields(name) {
                    expected_fields.iter().all(|(exp_name, exp_type)| {
                        actual_fields.iter().any(|(self_name, self_type)| {
                            self_name == exp_name && self_type.is_subtype_of(exp_type)
                        })
                    })
                } else {
                    false
                }
            }
            // BuchiPack vs Named: resolve Named and check structurally
            (Type::BuchiPack(actual_fields), Type::Named(name)) => {
                if let Some(expected_fields) = self.get_type_fields(name) {
                    expected_fields.iter().all(|(exp_name, exp_type)| {
                        actual_fields.iter().any(|(self_name, self_type)| {
                            self_name == exp_name && self_type.is_subtype_of(exp_type)
                        })
                    })
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Resolve a type expression to a concrete Type.
    ///
    /// N-74: This method does not cache results. Each call re-traverses the
    /// TypeExpr tree. This is acceptable at current codebase scale because:
    /// 1. Type expressions are typically shallow (1-3 levels deep).
    /// 2. The checker calls resolve_type() O(n) times per program where n is
    ///    the number of type annotations — not per-expression.
    /// 3. Adding a cache would require either interior mutability (&self -> &mut self
    ///    propagation) or a RefCell, adding complexity for negligible benefit.
    /// If profiling reveals this as a bottleneck, consider a HashMap<TypeExpr, Type>
    /// cache with the TypeExpr implementing Hash + Eq.
    pub fn resolve_type(&self, ty: &crate::parser::TypeExpr) -> Type {
        use crate::parser::TypeExpr;
        match ty {
            TypeExpr::Named(name) => match name.as_str() {
                "Int" => Type::Int,
                "Float" => Type::Float,
                "Num" | "Number" => Type::Num,
                "Str" | "String" => Type::Str,
                "Bytes" => Type::Bytes,
                "Bool" | "Boolean" => Type::Bool,
                "JSON" => Type::Json,
                "Molten" => Type::Molten,
                other => {
                    if self.is_error_type(other) {
                        Type::Error(other.to_string())
                    } else {
                        Type::Named(other.to_string())
                    }
                }
            },
            TypeExpr::BuchiPack(fields) => {
                let resolved: Vec<(String, Type)> = fields
                    .iter()
                    .map(|f| {
                        let field_type = f
                            .type_annotation
                            .as_ref()
                            .map(|t| self.resolve_type(t))
                            .unwrap_or(Type::Unknown);
                        (f.name.clone(), field_type)
                    })
                    .collect();
                Type::BuchiPack(resolved)
            }
            TypeExpr::List(inner) => Type::List(Box::new(self.resolve_type(inner))),
            TypeExpr::Generic(name, args) => {
                let resolved_args: Vec<Type> = args.iter().map(|a| self.resolve_type(a)).collect();
                Type::Generic(name.clone(), resolved_args)
            }
            TypeExpr::Function(params, ret) => {
                let resolved_params: Vec<Type> =
                    params.iter().map(|p| self.resolve_type(p)).collect();
                Type::Function(resolved_params, Box::new(self.resolve_type(ret)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_values_exist() {
        // All types must have default values — PHILOSOPHY.md constraint
        let types = vec![
            Type::Int,
            Type::Float,
            Type::Num,
            Type::Str,
            Type::Bytes,
            Type::Bool,
            Type::BuchiPack(vec![]),
            Type::List(Box::new(Type::Int)),
            Type::Unit,
            Type::Json,
            Type::Molten,
        ];
        for ty in types {
            assert!(
                !ty.default_value_description().is_empty(),
                "Type {} must have a default value",
                ty
            );
        }
    }

    #[test]
    fn test_numeric_subtyping() {
        assert!(Type::Int.is_subtype_of(&Type::Num));
        assert!(Type::Float.is_subtype_of(&Type::Num));
        assert!(Type::Int.is_subtype_of(&Type::Float));
        assert!(!Type::Str.is_subtype_of(&Type::Num));
    }

    #[test]
    fn test_structural_subtyping() {
        // Employee (name, age, dept) is subtype of Person (name, age)
        let person = Type::BuchiPack(vec![
            ("name".to_string(), Type::Str),
            ("age".to_string(), Type::Int),
        ]);
        let employee = Type::BuchiPack(vec![
            ("name".to_string(), Type::Str),
            ("age".to_string(), Type::Int),
            ("department".to_string(), Type::Str),
        ]);
        assert!(employee.is_subtype_of(&person));
        assert!(!person.is_subtype_of(&employee));
    }

    #[test]
    fn test_error_subtyping() {
        let base_error = Type::Error("Error".to_string());
        let custom_error = Type::Error("ValidationError".to_string());
        assert!(custom_error.is_subtype_of(&base_error));
    }

    #[test]
    fn test_list_covariance() {
        let int_list = Type::List(Box::new(Type::Int));
        let num_list = Type::List(Box::new(Type::Num));
        assert!(int_list.is_subtype_of(&num_list));
        assert!(!num_list.is_subtype_of(&int_list));
    }

    #[test]
    fn test_type_registry() {
        let mut reg = TypeRegistry::new();

        // Register Person type
        reg.register_type(
            "Person",
            vec![
                ("name".to_string(), Type::Str),
                ("age".to_string(), Type::Int),
            ],
        );

        // Register Employee inheriting from Person
        reg.register_inheritance(
            "Person",
            "Employee",
            vec![("department".to_string(), Type::Str)],
        );

        let emp_fields = reg
            .get_type_fields("Employee")
            .expect("Employee type should be registered after inheritance");
        assert_eq!(emp_fields.len(), 3);

        // Register error type
        reg.register_error_type("ValidationError", vec![("field".to_string(), Type::Str)]);
        assert!(reg.is_error_type("ValidationError"));
        assert!(reg.is_error_type("Error"));
        assert!(!reg.is_error_type("Person"));
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", Type::Int), "Int");
        assert_eq!(format!("{}", Type::List(Box::new(Type::Str))), "@[Str]");
        assert_eq!(format!("{}", Type::Bytes), "Bytes");
        assert_eq!(
            format!(
                "{}",
                Type::BuchiPack(vec![
                    ("name".to_string(), Type::Str),
                    ("age".to_string(), Type::Int),
                ])
            ),
            "@(name: Str, age: Int)"
        );
        assert_eq!(
            format!("{}", Type::Generic("Optional".to_string(), vec![Type::Int])),
            "Optional[Int]"
        );
        assert_eq!(format!("{}", Type::Molten), "Molten");
    }

    #[test]
    fn test_named_structural_subtyping_via_registry() {
        let mut reg = TypeRegistry::new();
        reg.register_type(
            "Person",
            vec![
                ("name".to_string(), Type::Str),
                ("age".to_string(), Type::Int),
            ],
        );
        reg.register_inheritance(
            "Person",
            "Employee",
            vec![("department".to_string(), Type::Str)],
        );

        let person = Type::Named("Person".to_string());
        let employee = Type::Named("Employee".to_string());

        // Employee is subtype of Person (inheritance)
        assert!(reg.is_subtype_of(&employee, &person));
        // Person is NOT subtype of Employee (missing department field)
        assert!(!reg.is_subtype_of(&person, &employee));
    }

    #[test]
    fn test_named_structural_subtyping_no_inheritance() {
        let mut reg = TypeRegistry::new();
        reg.register_type(
            "Point2D",
            vec![("x".to_string(), Type::Int), ("y".to_string(), Type::Int)],
        );
        reg.register_type(
            "Point3D",
            vec![
                ("x".to_string(), Type::Int),
                ("y".to_string(), Type::Int),
                ("z".to_string(), Type::Int),
            ],
        );

        let p2d = Type::Named("Point2D".to_string());
        let p3d = Type::Named("Point3D".to_string());

        // Point3D is structurally a subtype of Point2D (has x, y, plus z)
        assert!(reg.is_subtype_of(&p3d, &p2d));
        // Point2D is NOT a subtype of Point3D (missing z)
        assert!(!reg.is_subtype_of(&p2d, &p3d));
    }

    #[test]
    fn test_named_vs_buchipack_subtyping() {
        let mut reg = TypeRegistry::new();
        reg.register_type(
            "Person",
            vec![
                ("name".to_string(), Type::Str),
                ("age".to_string(), Type::Int),
            ],
        );

        let person = Type::Named("Person".to_string());
        let bp = Type::BuchiPack(vec![("name".to_string(), Type::Str)]);

        // Person is subtype of @(name: Str) (has name + age)
        assert!(reg.is_subtype_of(&person, &bp));
        // @(name: Str) is NOT subtype of Person (missing age)
        assert!(!reg.is_subtype_of(&bp, &person));
    }
}
