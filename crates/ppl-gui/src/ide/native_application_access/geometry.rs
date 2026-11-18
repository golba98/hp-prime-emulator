//! Measured Geometry Instruction/object access, committed only by the UI fence.
mod host;
use crate::ide::native_geometry::{Book, Object};
pub(crate) use host::read_only_host;

use ppl_vm::{
    ErrorKind, PortableValue as P, RuntimeError, SessionSnapshot, host::AppVariableRequest,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(crate) type ReadSnapshot = Option<Arc<Book>>;

fn invalid(message: impl Into<String>) -> RuntimeError {
    RuntimeError::new(ErrorKind::InvalidInput, message.into())
}
fn number(n: i64) -> P {
    P::Real(ppl_math::Real::from_i64(n))
}
fn symbol(s: impl Into<String>) -> P {
    P::Symbolic(ppl_cas::CasValue::Symbolic(s.into()))
}
fn check(cancel: &AtomicBool) -> Result<(), RuntimeError> {
    if cancel.load(Ordering::Acquire) {
        Err(RuntimeError::kind(ErrorKind::Interrupted))
    } else {
        Ok(())
    }
}
fn integer(value: &P) -> Result<i64, RuntimeError> {
    let value = value.to_value();
    let value = value.expect_real("Geometry instruction index")?;
    if !value.is_integer() {
        return Err(invalid("Geometry instruction index must be an integer"));
    }
    value
        .to_i64()
        .ok_or_else(|| invalid("Geometry instruction index exceeds range"))
}
fn row(book: &Book, value: &P) -> Result<usize, RuntimeError> {
    match value {
        P::String(name) | P::Symbolic(ppl_cas::CasValue::Symbolic(name)) => book
            .objects
            .iter()
            .position(|o| name == &format!("G{}", o.name))
            .ok_or_else(|| invalid("Unknown Geometry instruction")),
        _ => {
            let n = integer(value)?;
            if n < 0 || book.objects.is_empty() {
                return Err(invalid("Invalid Geometry instruction index"));
            }
            Ok((n.max(1) as usize - 1).min(book.objects.len() - 1))
        }
    }
}
fn definition(source: &str, snapshot: &SessionSnapshot) -> Result<P, RuntimeError> {
    // Numeric constants retain their calculator type; constructions remain
    // syntax, without calculating references or changing the cached value.
    if let Some(value) = ppl_math::Real::parse_versioned(source, snapshot.settings.numeric_version)
    {
        return Ok(P::Real(value));
    }
    Ok(symbol(source))
}
fn text(object: &Object) -> String {
    format!(
        "G{}:= {}; // c(FF{:06X}) v({}){} \n",
        object.name,
        object.source,
        object.color & 0xffffff,
        u8::from(object.visible),
        if object.traced { " t(1)" } else { "" }
    )
}
fn metadata(book: &Book, index: usize, snapshot: &SessionSnapshot) -> Result<Vec<P>, RuntimeError> {
    let object = &book.objects[index];
    let color = format!("FF{:06X}h", object.color & 0xffffff);
    Ok(vec![
        P::String(format!("G{}", object.name)),
        definition(&object.source, snapshot)?,
        book.current_values
            .get(&object.name)
            .cloned()
            .unwrap_or(P::Symbolic(ppl_cas::CasValue::Undefined)),
        P::Integer(
            ppl_math::HpInt::parse_literal_with_defaults(&color, ppl_math::IntBase::Hex, 32)
                .ok_or_else(|| invalid("Invalid Geometry color"))?,
        ),
        number(i64::from(object.visible)),
        number(4),
        number(0),
        number(i64::from(object.traced)),
        number(i64::from(object.label)),
        P::List(vec![]),
        P::List(vec![]),
    ])
}
fn assigned_source(value: &P) -> Result<String, RuntimeError> {
    let source = match value {
        P::String(source) | P::Symbolic(ppl_cas::CasValue::Symbolic(source)) => source.clone(),
        P::Real(_) | P::Integer(_) | P::Complex(_) => ppl_vm::cas::expression(&value.to_value())?,
        _ => {
            return Err(RuntimeError::new(
                ErrorKind::BadArgType,
                "Geometry definition needs an expression",
            ));
        }
    };
    let source = source.trim().trim_end_matches(';').trim();
    // The original accepts an incomplete single argument as NULL. Do not
    // reject definitions merely because their construction cannot yet run.
    let source = if source.ends_with('(')
        && source[..source.len() - 1]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        format!("{source}NULL)")
    } else {
        source.to_owned()
    };
    if source.is_empty() || source.len() > 4096 {
        return Err(invalid("Geometry definition needs 1 to 4096 bytes"));
    }
    let (_, diagnostics) = ppl_syntax::parse_expression(&source);
    if diagnostics.has_errors() {
        return Err(invalid(
            diagnostics.errors().next().unwrap().message.clone(),
        ));
    }
    Ok(source)
}
/// Read a previously validated Geometry snapshot without dispatching mutations.
pub(crate) fn read_only(
    book: &Book,
    request: &AppVariableRequest,
    snapshot: &SessionSnapshot,
    cancel: Arc<AtomicBool>,
) -> Result<P, RuntimeError> {
    check(&cancel)?;
    if request.app != "Geometry" {
        return Err(invalid("Geometry variables require the Geometry app"));
    }
    if request.value.is_some() || request.name.eq_ignore_ascii_case("DelInstruction") {
        return Err(RuntimeError::new(
            ErrorKind::Unsupported,
            "Read-only expressions cannot change Geometry objects or instructions",
        ));
    }
    let value = match request.name.as_str() {
        "__Expression" => {
            let Some((P::Symbolic(ppl_cas::CasValue::Symbolic(source)), captures)) =
                request.indices.split_last()
            else {
                return Err(invalid("Invalid Geometry expression request"));
            };
            let mut frame = snapshot.clone();
            for capture in captures {
                let P::List(pair) = capture else {
                    return Err(invalid("Invalid Geometry argument capture"));
                };
                let [P::String(name), value] = pair.as_slice() else {
                    return Err(invalid("Invalid Geometry argument binding"));
                };
                if !name.starts_with("__GeometryArgument") {
                    return Err(invalid("Invalid Geometry argument name"));
                }
                frame.globals.insert(name.clone(), value.clone());
            }
            book.current_expression(source, &frame, cancel.clone())
                .map_err(invalid)?
        }
        "Instruction" => match request.indices.as_slice() {
            [] => P::String(book.objects.iter().map(text).collect()),
            [target] => definition(&book.objects[row(book, target)?].source, snapshot)?,
            [target, field] => {
                let index = row(book, target)?;
                match integer(field)? {
                    0 => P::String(text(&book.objects[index])),
                    1 => P::List(metadata(book, index, snapshot)?),
                    field @ 2..=12 => metadata(book, index, snapshot)?.remove(field as usize - 2),
                    _ => return Err(invalid("Invalid Geometry metadata field")),
                }
            }
            _ => return Err(invalid("Invalid Geometry instruction arguments")),
        },
        name if name.starts_with('G') => {
            let object = requested_object(book, &name[1..], request)?;
            if let Some(value) = book.current_values.get(&object.name) {
                value.clone()
            } else {
                definition(&object.source, snapshot)?
            }
        }
        _ => return Err(invalid("Unknown Geometry variable")),
    };
    check(&cancel)?;
    Ok(value)
}

fn requested_object<'a>(
    book: &'a Book,
    name: &str,
    request: &AppVariableRequest,
) -> Result<&'a Object, RuntimeError> {
    if !request.indices.is_empty() {
        return Err(invalid("Geometry objects cannot be indexed"));
    }
    book.objects
        .iter()
        .find(|o| o.name == name)
        .ok_or_else(|| invalid("Unknown Geometry object"))
}

pub fn access(
    mut book: Book,
    request: &AppVariableRequest,
    snapshot: &SessionSnapshot,
    cancel: Arc<AtomicBool>,
) -> Result<(P, Option<Book>), RuntimeError> {
    check(&cancel)?;
    if request.app != "Geometry" {
        return Err(invalid("Geometry variables require the Geometry app"));
    }
    book.validate().map_err(invalid)?;
    if request.value.is_none() && request.name != "DelInstruction" {
        return Ok((read_only(&book, request, snapshot, cancel)?, None));
    }
    let value = match (request.name.as_str(), &request.value) {
        ("__Expression", Some(_)) => {
            return Err(invalid("Geometry expressions cannot be assigned"));
        }
        ("Instruction", Some(value)) => {
            let [target] = request.indices.as_slice() else {
                return Err(invalid("Invalid Geometry instruction assignment"));
            };
            let source = assigned_source(value)?;
            let n = match target {
                P::String(_) | P::Symbolic(_) => None,
                _ => Some(integer(target)?),
            };
            if n.is_some_and(|n| n <= 0 || n as usize > book.objects.len()) {
                if book.objects.len() >= crate::ide::native_geometry::MAX_OBJECTS {
                    return Err(invalid("Geometry object limit exceeded"));
                }
                let name = book.next_name();
                let object = Object {
                    name,
                    source,
                    ..Default::default()
                };
                // Installed G1 inserts negative-index writes immediately
                // before the last instruction, including -1 and -2.
                let at = if n.is_some_and(|n| n < 0) {
                    book.objects.len().saturating_sub(1)
                } else {
                    book.objects.len()
                };
                book.objects.insert(at, object);
            } else {
                let index = row(&book, target)?;
                book.objects[index].source = source;
            }
            number(1)
        }
        ("DelInstruction", None) => {
            match request.indices.as_slice() {
                [] => {
                    book.objects.clear();
                    book.current_values.clear();
                    book.current_point_curves.clear();
                    book.current_hyperbolas.clear();
                }
                [a] | [a, _] => {
                    let a = integer(a)?;
                    let b = match request.indices.get(1) {
                        Some(b) => integer(b)?,
                        None => a,
                    };
                    if a < 1 || b < a || b as usize > book.objects.len() {
                        return Err(invalid("Invalid Geometry deletion range"));
                    }
                    for object in book.objects.drain(a as usize - 1..b as usize) {
                        book.current_values.remove(&object.name);
                        book.current_point_curves.remove(&object.name);
                        book.current_hyperbolas.remove(&object.name);
                    }
                }
                _ => return Err(invalid("Invalid Geometry deletion arguments")),
            }
            number(1)
        }
        ("DelInstruction", Some(_)) => {
            return Err(invalid("DelInstruction cannot be assigned"));
        }
        (name, Some(value)) if name.starts_with('G') => {
            let object = requested_object(&book, &name[1..], request)?;
            let name = object.name.clone();
            book.current_point_curves.remove(&name);
            book.current_hyperbolas.remove(&name);
            book.current_values.insert(name, value.clone());
            value.clone()
        }
        _ => return Err(invalid("Unknown Geometry variable")),
    };
    check(&cancel)?;
    book.validate().map_err(invalid)?;
    Ok((value, Some(book)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trace_metadata_reads_saved_flag_and_instruction_text() {
        let mut object: Object =
            serde_json::from_str(r#"{"name":"A","source":"point(1,2)"}"#).unwrap();
        assert!(!object.traced);
        assert!(!text(&object).contains(" t(1)"));
        object.traced = true;
        let saved = serde_json::to_string(&object).unwrap();
        let mut book = Book::default();
        book.objects.push(serde_json::from_str(&saved).unwrap());
        let cancel = Arc::new(AtomicBool::new(false));
        let state = snapshot();
        assert_eq!(
            read_only(
                &book,
                &request("Instruction", vec![number(1), number(9)], None),
                &state,
                cancel.clone()
            )
            .unwrap(),
            number(1)
        );
        assert_eq!(metadata(&book, 0, &state).unwrap()[7], number(1));
        assert_eq!(
            text(&book.objects[0]),
            "GA:= point(1,2); // c(FF000000) v(1) t(1) \n"
        );
        assert!(
            access(
                book,
                &request("Instruction", vec![number(1), number(9)], Some(number(0))),
                &state,
                cancel
            )
            .is_err()
        );
    }
    fn snapshot() -> SessionSnapshot {
        ppl_vm::SessionContext::default().snapshot()
    }
    fn request(name: &str, indices: Vec<P>, value: Option<P>) -> AppVariableRequest {
        AppVariableRequest {
            app: "Geometry".into(),
            name: name.into(),
            indices,
            value,
        }
    }
    fn call(
        book: &mut Book,
        name: &str,
        indices: Vec<P>,
        value: Option<P>,
    ) -> Result<P, RuntimeError> {
        let (value, changed) = access(
            book.clone(),
            &request(name, indices, value),
            &snapshot(),
            Arc::new(AtomicBool::new(false)),
        )?;
        if let Some(changed) = changed {
            *book = changed;
        }
        Ok(value)
    }
    fn append(book: &mut Book, source: &str) {
        assert_eq!(
            call(
                book,
                "Instruction",
                vec![number(0)],
                Some(P::String(source.into()))
            )
            .unwrap(),
            number(1)
        );
    }
    fn expression(book: &Book, source: &str, snapshot: &SessionSnapshot) -> P {
        access(
            book.clone(),
            &request("__Expression", vec![symbol(source)], None),
            snapshot,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap()
        .0
    }
    fn triangle() -> Book {
        let mut book = Book::default();
        for source in [
            "point(0,0)",
            "point(4,0)",
            "point(0,3)",
            "triangle(GA,GB,GC)",
            "distance(GB,GC)",
        ] {
            append(&mut book, source);
        }
        book
    }
    #[test]
    fn definitions_and_current_values_have_distinct_redraw_and_profile_lifetimes() {
        let mut book = triangle();
        assert_eq!(
            call(&mut book, "Instruction", vec![number(5), number(4)], None).unwrap(),
            P::Symbolic(ppl_cas::CasValue::Undefined)
        );
        assert_eq!(
            call(&mut book, "GE", vec![], None).unwrap(),
            symbol("distance(GB,GC)")
        );
        book.refresh_values(&snapshot(), Arc::new(AtomicBool::new(false)))
            .unwrap();
        assert_eq!(call(&mut book, "GE", vec![], None).unwrap(), number(5));
        let before = book.access_fingerprint();
        assert_eq!(
            call(&mut book, "GA", vec![], Some(symbol("point(9,12)"))).unwrap(),
            symbol("point(9,12)")
        );
        assert_ne!(before, book.access_fingerprint());
        assert_eq!(
            call(&mut book, "Instruction", vec![number(1)], None).unwrap(),
            symbol("point(0,0)")
        );
        assert_eq!(expression(&book, "distance(GA,0)", &snapshot()), number(15));
        call(
            &mut book,
            "Instruction",
            vec![number(1)],
            Some(P::String("point(6,8)".into())),
        )
        .unwrap();
        assert_eq!(
            call(&mut book, "GA", vec![], None).unwrap(),
            symbol("point(9,12)")
        );
        assert_eq!(
            call(&mut book, "Instruction", vec![number(1)], None).unwrap(),
            symbol("point(6,8)")
        );
        let mut reopened: Book =
            serde_json::from_value(serde_json::to_value(&book).unwrap()).unwrap();
        assert!(reopened.current_values.is_empty());
        assert_eq!(
            call(&mut reopened, "GA", vec![], None).unwrap(),
            symbol("point(6,8)")
        );
        reopened
            .refresh_values(&snapshot(), Arc::new(AtomicBool::new(false)))
            .unwrap();
        assert_eq!(
            expression(&reopened, "distance(Geometry.GA,0)", &snapshot()),
            number(10)
        );
    }
    #[test]
    fn actual_constructions_signed_area_and_typed_coordinates_use_shared_book() {
        let mut book = triangle();
        book.refresh_values(&snapshot(), Arc::new(AtomicBool::new(false)))
            .unwrap();
        assert_eq!(expression(&book, "area(GD)", &snapshot()), number(6));
        assert_eq!(expression(&book, "perimeter(GD)", &snapshot()), number(12));
        assert_eq!(
            expression(&book, "coordinates(GB)", &snapshot()),
            P::Matrix {
                rows: 1,
                cols: 2,
                vector: true,
                data: vec![number(4), number(0)]
            }
        );
        call(
            &mut book,
            "Instruction",
            vec![number(2)],
            Some(P::String("point(0,3)".into())),
        )
        .unwrap();
        call(
            &mut book,
            "Instruction",
            vec![number(3)],
            Some(P::String("point(4,0)".into())),
        )
        .unwrap();
        book.refresh_values(&snapshot(), Arc::new(AtomicBool::new(false)))
            .unwrap();
        assert_eq!(
            expression(&book, "area(Geometry.GD)", &snapshot()),
            number(-6)
        );
        assert_eq!(
            expression(&book, "perimeter(Geometry.GD)", &snapshot()),
            number(12)
        );
        let scene = book
            .calculate(&snapshot(), Arc::new(AtomicBool::new(false)))
            .unwrap();
        assert!(matches!(
            &scene.objects[4].value,
            Ok(crate::ide::native_geometry::Shape::Number { re: 5., im: 0. })
        ));
    }
    #[test]
    fn copied_symbolic_points_and_shadowing_locals_do_not_become_live_aliases() {
        let mut book = triangle();
        book.refresh_values(&snapshot(), Arc::new(AtomicBool::new(false)))
            .unwrap();
        let mut local = snapshot();
        local.globals.insert("GA".into(), number(7));
        local.globals.insert("p".into(), symbol("point(9,12)"));
        assert_eq!(expression(&book, "distance(GA,0)", &local), number(7));
        assert_eq!(
            expression(&book, "distance(Geometry.GA,0)", &local),
            number(0)
        );
        let copied = expression(&book, "distance(p,0)", &local);
        assert!(
            matches!(copied, P::Symbolic(ppl_cas::CasValue::Symbolic(s)) if s.contains("point(9,12)") && s.contains("re("))
        );
        assert_eq!(
            expression(&book, "distance(point(9,12),0)", &local),
            number(15)
        );
        assert_eq!(
            expression(&book, "coordinates(p)", &local),
            P::List(vec![symbol("point(9,0)"), symbol("point(12,0)")])
        );
    }
    #[test]
    fn instruction_metadata_and_measured_index_controls_are_atomic() {
        let mut book = Book::default();
        for source in ["point(1,1)", "point(2,2)", "point(3,3)"] {
            append(&mut book, source);
        }
        call(
            &mut book,
            "Instruction",
            vec![number(-1)],
            Some(P::String("point(9,9)".into())),
        )
        .unwrap();
        assert_eq!(
            book.objects
                .iter()
                .map(|o| o.name.as_str())
                .collect::<Vec<_>>(),
            ["A", "B", "D", "C"]
        );
        call(
            &mut book,
            "Instruction",
            vec![number(-2)],
            Some(P::String("point(8,8)".into())),
        )
        .unwrap();
        assert_eq!(
            book.objects
                .iter()
                .map(|o| o.name.as_str())
                .collect::<Vec<_>>(),
            ["A", "B", "D", "E", "C"]
        );
        assert_eq!(
            call(&mut book, "Instruction", vec![number(999), number(2)], None).unwrap(),
            P::String("GC".into())
        );
        assert_eq!(
            call(&mut book, "Instruction", vec![number(0), number(2)], None).unwrap(),
            P::String("GA".into())
        );
        let before = book.access_fingerprint();
        assert!(
            call(
                &mut book,
                "Instruction",
                vec![number(1), number(6)],
                Some(number(0))
            )
            .is_err()
        );
        assert!(
            call(
                &mut book,
                "DelInstruction",
                vec![P::String("GD".into())],
                None
            )
            .is_err()
        );
        assert_eq!(before, book.access_fingerprint());
        call(
            &mut book,
            "DelInstruction",
            vec![number(2), number(3)],
            None,
        )
        .unwrap();
        assert_eq!(
            book.objects
                .iter()
                .map(|o| o.name.as_str())
                .collect::<Vec<_>>(),
            ["A", "E", "C"]
        );
        call(
            &mut book,
            "Instruction",
            vec![number(1)],
            Some(P::String("point(".into())),
        )
        .unwrap();
        assert_eq!(book.objects[0].source, "point(NULL)");
        call(&mut book, "Instruction", vec![number(1)], Some(number(42))).unwrap();
        assert_eq!(
            call(&mut book, "Instruction", vec![P::String("GA".into())], None).unwrap(),
            number(42)
        );
        call(&mut book, "DelInstruction", vec![], None).unwrap();
        assert!(book.objects.is_empty());
    }
}
