//! Geometry's lowercase symbolic functions are distinct from uppercase graphics
//! commands such as TRIANGLE. Arguments retain source and live caller bindings.
pub fn is_object(name: &str) -> bool {
    name.strip_prefix('G').is_some_and(|name| {
        !name.is_empty()
            && name.len() <= 24
            && name.as_bytes()[0].is_ascii_uppercase()
            && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
    })
}
pub fn is_variable(name: &str) -> bool {
    is_object(name) || matches!(name, "Instruction" | "DelInstruction") || is_plot_field(name)
}
pub fn is_plot_field(name: &str) -> bool {
    [
        "Xmin",
        "Ymin",
        "PixSize",
        "Xtick",
        "Ytick",
        "Axes",
        "GridDots",
        "GridLines",
        "Labels",
        "ScrollText",
        "Xmax",
        "Ymax",
    ]
    .iter()
    .any(|n| n.eq_ignore_ascii_case(name))
}
pub fn is_function(name: &str) -> bool {
    if name.starts_with("Geometry.") {
        return false;
    }
    // Preserve the public graphics builtins and their case-insensitive aliases.
    // Lowercase geometry spellings opt in; only variables use app qualifiers.
    if name != name.to_ascii_lowercase() {
        return false;
    }
    matches!(
        name.to_ascii_lowercase().as_str(),
        "point"
            | "element"
            | "tangent"
            | "segment"
            | "line"
            | "half_line"
            | "midpoint"
            | "parallel"
            | "perpendicular"
            | "perpen_bisector"
            | "median_line"
            | "altitude"
            | "bisector"
            | "inter"
            | "single_inter"
            | "circle"
            | "ellipse"
            | "circumcircle"
            | "incircle"
            | "triangle"
            | "polygon"
            | "center"
            | "radius"
            | "abscissa"
            | "ordinate"
            | "affix"
            | "distance"
            | "area"
            | "perimeter"
            | "slope"
            | "angle"
            | "projection"
            | "reflection"
            | "rotation"
            | "homothety"
            | "coordinates"
    )
}
pub fn is_function_call(name: &str, _argc: usize) -> bool {
    // These are PPL graphics commands even in the Geometry app. Geometry's
    // definition parser handles their construction forms separately.
    if matches!(name, "line" | "triangle") {
        return false;
    }
    is_function(name)
}
