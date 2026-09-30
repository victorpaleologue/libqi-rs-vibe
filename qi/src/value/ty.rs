/// The type of a value in the `qi` type system.
///
/// The absence of a type equals to the unit `Dynamic` type, which is the set of all types.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Type {
    Unit,
    Bool,
    Int8,
    UInt8,
    Int16,
    UInt16,
    Int32,
    UInt32,
    Int64,
    UInt64,
    Float32,
    Float64,
    String,
    Raw,
    Object,
    Option(Option<Box<Type>>),
    List(Option<Box<Type>>),
    VarArgs(Option<Box<Type>>),
    Map {
        key: Option<Box<Type>>,
        value: Option<Box<Type>>,
    },
    Tuple(Tuple),
}

impl Type {
    pub fn list_of<T>(t: T) -> Self
    where
        T: Into<Option<Type>>,
    {
        Self::List(t.into().map(Box::new))
    }

    pub fn option_of<T>(t: T) -> Self
    where
        T: Into<Option<Type>>,
    {
        Self::Option(t.into().map(Box::new))
    }

    pub fn varargs_of<T>(t: T) -> Self
    where
        T: Into<Option<Type>>,
    {
        Self::VarArgs(t.into().map(Box::new))
    }

    pub fn map_of<K, V>(key: K, value: V) -> Self
    where
        K: Into<Option<Type>>,
        V: Into<Option<Type>>,
    {
        Self::Map {
            key: key.into().map(Box::new),
            value: value.into().map(Box::new),
        }
    }

    pub fn tuple_of<I, F>(fields: I) -> Self
    where
        I: IntoIterator<Item = F>,
        F: Into<Option<Type>>,
    {
        Self::Tuple(Tuple::Tuple(fields.into_iter().map(Into::into).collect()))
    }

    pub fn unit_tuple() -> Self {
        Self::Tuple(Tuple::Tuple(vec![]))
    }

    pub fn struct_of<N, I, F>(name: N, fields: I) -> Type
    where
        N: Into<String>,
        I: IntoIterator<Item = F>,
        F: Into<StructField>,
    {
        Type::Tuple(Tuple::Struct {
            name: name.into(),
            fields: fields.into_iter().map(Into::into).collect(),
        })
    }

    pub fn tuple_struct_of<N, I, F>(name: N, elements: I) -> Type
    where
        N: Into<String>,
        I: IntoIterator<Item = F>,
        F: Into<Option<Type>>,
    {
        Type::Tuple(Tuple::TupleStruct {
            name: name.into(),
            elements: elements.into_iter().map(Into::into).collect(),
        })
    }
}

/// Defaults constructs a type as a unit type.
impl Default for Type {
    fn default() -> Self {
        Self::Unit
    }
}

impl From<Tuple> for Type {
    fn from(tuple: Tuple) -> Self {
        Type::Tuple(tuple)
    }
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Type::Unit => f.write_str("unit"),
            Type::Bool => f.write_str("bool"),
            Type::Int8 => f.write_str("int8"),
            Type::UInt8 => f.write_str("uint8"),
            Type::Int16 => f.write_str("int16"),
            Type::UInt16 => f.write_str("uint16"),
            Type::Int32 => f.write_str("int32"),
            Type::UInt32 => f.write_str("uint32"),
            Type::Int64 => f.write_str("int64"),
            Type::UInt64 => f.write_str("uint64"),
            Type::Float32 => f.write_str("float32"),
            Type::Float64 => f.write_str("float64"),
            Type::String => f.write_str("string"),
            Type::Raw => f.write_str("raw"),
            Type::Object => f.write_str("object"),
            Type::Option(t) => {
                f.write_str("option(")?;
                DisplayOption(&t.as_deref()).fmt(f)?;
                f.write_str(")")
            }
            Type::List(t) => {
                f.write_str("list(")?;
                DisplayOption(&t.as_deref()).fmt(f)?;
                f.write_str(")")
            }
            Type::VarArgs(t) => {
                f.write_str("varargs(")?;
                DisplayOption(&t.as_deref()).fmt(f)?;
                f.write_str(")")
            }
            Type::Map { key, value } => {
                f.write_str("map(")?;
                DisplayOption(&key.as_deref()).fmt(f)?;
                f.write_str(",")?;
                DisplayOption(&value.as_deref()).fmt(f)?;
                f.write_str(")")
            }
            Type::Tuple(t) => t.fmt(f),
        }
    }
}

#[derive(Debug)]
pub struct DisplayOption<'a>(pub &'a Option<&'a Type>);

impl std::fmt::Display for DisplayOption<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Some(ty) => ty.fmt(f),
            None => f.write_str("dynamic"),
        }
    }
}

#[derive(Debug)]
pub struct DisplayTuple<'a>(pub &'a Vec<Option<&'a Type>>);

impl std::fmt::Display for DisplayTuple<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(")?;
        for t in self.0 {
            DisplayOption(t).fmt(f)?;
        }
        f.write_str(")")
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Tuple {
    Tuple(Vec<Option<Type>>),
    TupleStruct {
        name: String,
        elements: Vec<Option<Type>>,
    },
    Struct {
        name: String,
        fields: Vec<StructField>,
    },
}

impl Tuple {
    pub fn new() -> Self {
        Self::Tuple(vec![])
    }

    pub fn struct_from_annotations_of_elements(
        annotations: StructAnnotations,
        elements: Vec<Option<Type>>,
    ) -> Result<Self, ZipStructFieldsSizeError> {
        let tuple = if let Some(field_names) = annotations.field_names {
            Tuple::Struct {
                name: annotations.name,
                fields: zip_struct_fields(field_names, elements)?,
            }
        } else {
            Tuple::TupleStruct {
                name: annotations.name,
                elements,
            }
        };
        Ok(tuple)
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Tuple(t) => t.len(),
            Self::TupleStruct { elements, .. } => elements.len(),
            Self::Struct { fields, .. } => fields.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Self::Tuple(t) => t.is_empty(),
            Self::TupleStruct { elements, .. } => elements.is_empty(),
            Self::Struct { fields, .. } => fields.is_empty(),
        }
    }

    pub fn element_types(&self) -> Vec<Option<Type>> {
        match self {
            Self::Tuple(t) => t.clone(),
            Self::TupleStruct { elements, .. } => elements.clone(),
            Self::Struct { fields, .. } => fields.iter().map(|field| field.ty.clone()).collect(),
        }
    }

    pub fn name(&self) -> Option<String> {
        match self {
            Self::Tuple(_) => None,
            Self::TupleStruct { name, .. } | Self::Struct { name, .. } => Some(name.clone()),
        }
    }

    pub fn field_names(&self) -> Option<Vec<String>> {
        match self {
            Self::Tuple(_) | Self::TupleStruct { .. } => None,
            Self::Struct { fields, .. } => {
                Some(fields.iter().map(|field| field.name.clone()).collect())
            }
        }
    }

    pub fn annotations(&self) -> Option<StructAnnotations> {
        match self {
            Self::Tuple(_) => None,
            Self::TupleStruct { name, .. } => Some(StructAnnotations {
                name: name.clone(),
                field_names: None,
            }),
            Self::Struct { name, fields } => Some(StructAnnotations {
                name: name.clone(),
                field_names: Some(fields.iter().map(|field| field.name.clone()).collect()),
            }),
        }
    }
}

impl Default for Tuple {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for Tuple {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("tuple(")?;
        for (idx, element) in self.element_types().into_iter().enumerate() {
            if idx > 0 {
                f.write_str(",")?;
            }
            DisplayOption(&element.as_ref()).fmt(f)?;
        }
        f.write_str(")")?;
        if let Some(annotations) = self.annotations() {
            annotations.fmt(f)?;
        }
        Ok(())
    }
}

impl<I, T> From<I> for Tuple
where
    I: IntoIterator<Item = T>,
    T: Into<Option<Type>>,
{
    fn from(iter: I) -> Self {
        Self::Tuple(iter.into_iter().map(Into::into).collect())
    }
}

#[derive(Default, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct StructField {
    pub name: String,
    pub ty: Option<Type>,
}

impl<S, T> From<(S, T)> for StructField
where
    S: Into<String>,
    T: Into<Option<Type>>,
{
    fn from(v: (S, T)) -> Self {
        Self {
            name: v.0.into(),
            ty: v.1.into(),
        }
    }
}

#[derive(Default, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct StructAnnotations {
    pub name: String,
    pub field_names: Option<Vec<String>>,
}

impl std::fmt::Display for StructAnnotations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<{name}", name = self.name)?;
        if let Some(fields) = &self.field_names {
            for (idx, field) in fields.iter().enumerate() {
                if idx > 0 {
                    f.write_str(",")?;
                }
                f.write_str(field)?;
            }
        }
        f.write_str(">")?;
        Ok(())
    }
}

pub(crate) fn zip_struct_fields<N, E>(
    names: N,
    elements: E,
) -> Result<Vec<StructField>, ZipStructFieldsSizeError>
where
    N: IntoIterator,
    N::Item: Into<String>,
    E: IntoIterator,
    E::Item: Into<Option<Type>>,
{
    let mut names = names.into_iter().fuse();
    let mut elements = elements.into_iter().fuse();
    let mut fields = Vec::new();
    loop {
        match (names.next(), elements.next()) {
            (Some(name), Some(element)) => fields.push(StructField {
                name: name.into(),
                ty: element.into(),
            }),
            (None, None) => break Ok(fields),
            (name, element) => {
                break Err(ZipStructFieldsSizeError {
                    name_count: fields.len() + if name.is_some() { 1 } else { 0 } + names.count(),
                    element_count: fields.len()
                        + if element.is_some() { 1 } else { 0 }
                        + elements.count(),
                })
            }
        }
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash, thiserror::Error)]
#[error("error zipping structure fields names and elements, got {name_count} names for {element_count} elements")]
pub struct ZipStructFieldsSizeError {
    pub name_count: usize,
    pub element_count: usize,
}

/// The common type of two types, `None` if they have none.
///
/// Unknown element types (`None` inside a container type, as the element type of an empty
/// list) unify with any element type: `[m]` and `[i]` have the common type `[i]`, so that the
/// runtime types of tuples holding empty and non-empty lists agree.
pub fn common_type(t1: Option<Type>, t2: Option<Type>) -> Option<Type> {
    t1.zip(t2).and_then(|(t1, t2)| unify(t1, t2))
}

fn unify(t1: Type, t2: Type) -> Option<Type> {
    Some(match (t1, t2) {
        (t1, t2) if t1 == t2 => t1,
        (Type::Option(e1), Type::Option(e2)) => Type::Option(unify_boxed(e1, e2)?),
        (Type::List(e1), Type::List(e2)) => Type::List(unify_boxed(e1, e2)?),
        (Type::VarArgs(e1), Type::VarArgs(e2)) => Type::VarArgs(unify_boxed(e1, e2)?),
        (Type::Map { key: k1, value: v1 }, Type::Map { key: k2, value: v2 }) => Type::Map {
            key: unify_boxed(k1, k2)?,
            value: unify_boxed(v1, v2)?,
        },
        (Type::Tuple(t1), Type::Tuple(t2)) => Type::Tuple(unify_tuple(t1, t2)?),
        _ => return None,
    })
}

fn unify_element(e1: Option<Type>, e2: Option<Type>) -> Option<Option<Type>> {
    match (e1, e2) {
        (None, known) | (known, None) => Some(known),
        (Some(e1), Some(e2)) => unify(e1, e2).map(Some),
    }
}

fn unify_boxed(e1: Option<Box<Type>>, e2: Option<Box<Type>>) -> Option<Option<Box<Type>>> {
    unify_element(e1.map(|e| *e), e2.map(|e| *e)).map(|e| e.map(Box::new))
}

fn unify_elements(e1: Vec<Option<Type>>, e2: Vec<Option<Type>>) -> Option<Vec<Option<Type>>> {
    if e1.len() != e2.len() {
        return None;
    }
    e1.into_iter()
        .zip(e2)
        .map(|(e1, e2)| unify_element(e1, e2))
        .collect()
}

fn unify_tuple(t1: Tuple, t2: Tuple) -> Option<Tuple> {
    Some(match (t1, t2) {
        (Tuple::Tuple(e1), Tuple::Tuple(e2)) => Tuple::Tuple(unify_elements(e1, e2)?),
        (
            Tuple::TupleStruct {
                name: n1,
                elements: e1,
            },
            Tuple::TupleStruct {
                name: n2,
                elements: e2,
            },
        ) if n1 == n2 => Tuple::TupleStruct {
            name: n1,
            elements: unify_elements(e1, e2)?,
        },
        (
            Tuple::Struct {
                name: n1,
                fields: f1,
            },
            Tuple::Struct {
                name: n2,
                fields: f2,
            },
        ) if n1 == n2 && f1.len() == f2.len() => {
            let fields = f1
                .into_iter()
                .zip(f2)
                .map(|(f1, f2)| {
                    (f1.name == f2.name).then_some(()).and_then(|()| {
                        Some(StructField {
                            name: f1.name,
                            ty: unify_element(f1.ty, f2.ty)?,
                        })
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            Tuple::Struct { name: n1, fields }
        }
        _ => return None,
    })
}

pub fn reduce_type<C>(c: C) -> Option<Type>
where
    C: IntoIterator<Item = Type>,
{
    c.into_iter().map(Some).reduce(common_type).flatten()
}

pub fn reduce_map_types<C>(c: C) -> (Option<Type>, Option<Type>)
where
    C: IntoIterator<Item = (Type, Type)>,
{
    c.into_iter()
        .map(|(k, v)| (Some(k), Some(v)))
        .reduce(|(ck, cv), (k, v)| (common_type(ck, k), common_type(cv, v)))
        .unwrap_or((None, None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_element_types_unify() {
        let empty_list = Type::List(None);
        let int_list = Type::List(Some(Box::new(Type::Int32)));
        assert_eq!(
            reduce_type([empty_list.clone(), int_list.clone(), empty_list.clone()]),
            Some(int_list.clone())
        );
        assert_eq!(
            reduce_type([empty_list.clone(), empty_list.clone()]),
            Some(empty_list.clone())
        );
        assert_eq!(
            reduce_type([int_list.clone(), Type::List(Some(Box::new(Type::String)))]),
            None
        );
        // Tuples holding empty and non-empty lists have a common type.
        let t1 = Type::Tuple(Tuple::Tuple(vec![Some(Type::Int32), Some(empty_list)]));
        let t2 = Type::Tuple(Tuple::Tuple(vec![
            Some(Type::Int32),
            Some(int_list.clone()),
        ]));
        assert_eq!(reduce_type([t1, t2.clone()]), Some(t2));
        assert_eq!(reduce_type([Type::Int32, Type::String]), None);
        assert_eq!(reduce_type([Type::Int32, Type::Int32]), Some(Type::Int32));
    }
}
