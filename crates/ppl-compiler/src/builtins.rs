//! The built-in command table.
//!
//! Every name the Prime is known to provide appears here once, with its
//! accepted argument counts and its implementation status in this emulator.
//! Arity ranges were seeded from the Python project's `hpprime_specs.py`
//! and cross-checked against every syntax line of the *HP Prime User
//! Guide*, 3rd ed. (Dec 2017), ch. 23–29: a documented form is never
//! rejected here, even when the emulator only implements part of it (the
//! unimplemented forms raise "unsupported" at run time). None are
//! hardware-verified yet.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Math,
    Bits,
    Strings,
    Lists,
    Matrices,
    Graphics,
    Io,
    Time,
    System,
    Cas,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Math => "math",
            Category::Bits => "bits",
            Category::Strings => "strings",
            Category::Lists => "lists",
            Category::Matrices => "matrices",
            Category::Graphics => "graphics",
            Category::Io => "io",
            Category::Time => "time",
            Category::System => "system",
            Category::Cas => "cas",
        }
    }
}

/// How faithfully a command is emulated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Implemented from documented behaviour; not yet compared with the G1.
    Implemented,
    /// Implemented, with known deviations recorded in the compatibility matrix.
    Approximate,
    /// Known Prime command that this emulator does not implement: running it
    /// is a runtime error, never a placeholder value.
    Unsupported,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Implemented => "implemented",
            Status::Approximate => "approximate",
            Status::Unsupported => "unsupported",
        }
    }
}

pub const VARIADIC: u8 = u8::MAX;

#[derive(Clone, Copy, Debug)]
pub struct BuiltinSpec {
    pub builtin: Builtin,
    pub name: &'static str,
    pub min: u8,
    pub max: u8,
    pub category: Category,
    pub status: Status,
    /// Leading arguments that are variables written by the command
    /// (`INPUT(var…)`, `CHOOSE(var,…)`).
    pub ref_args: u8,
}

impl BuiltinSpec {
    pub fn accepts(&self, n: usize) -> bool {
        n >= self.min as usize && (self.max == VARIADIC || n <= self.max as usize)
    }

    pub fn arity_text(&self) -> String {
        match (self.min, self.max) {
            (a, b) if a == b => a.to_string(),
            (a, VARIADIC) => format!("{a} or more"),
            (a, b) => format!("{a}-{b}"),
        }
    }
}

macro_rules! builtins {
    ($( $variant:ident $name:literal $min:literal ..= $max:tt $cat:ident $status:ident $(ref $refs:literal)? ;)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(u16)]
        pub enum Builtin { $($variant),* }

        pub static BUILTINS: &[BuiltinSpec] = &[
            $(BuiltinSpec {
                builtin: Builtin::$variant,
                name: $name,
                min: $min,
                max: builtins!(@max $max),
                category: Category::$cat,
                status: Status::$status,
                ref_args: builtins!(@refs $($refs)?),
            }),*
        ];
    };
    (@max N) => { VARIADIC };
    (@max $m:literal) => { $m };
    (@refs) => { 0 };
    (@refs $r:literal) => { $r };
}

builtins! {
    // ---- math ----
    Abs "ABS" 1..=1 Math Implemented;
    Sqrt "SQRT" 1..=1 Math Implemented;
    Sq "SQ" 1..=1 Math Implemented;
    Exp "EXP" 1..=1 Math Implemented;
    Ln "LN" 1..=1 Math Implemented;
    Log "LOG" 1..=2 Math Implemented;
    Alog "ALOG" 1..=1 Math Implemented;
    Expm1 "EXPM1" 1..=1 Math Implemented;
    Lnp1 "LNP1" 1..=1 Math Implemented;
    Sin "SIN" 1..=1 Math Implemented;
    Cos "COS" 1..=1 Math Implemented;
    Tan "TAN" 1..=1 Math Implemented;
    Asin "ASIN" 1..=1 Math Implemented;
    Acos "ACOS" 1..=1 Math Implemented;
    Atan "ATAN" 1..=1 Math Implemented;
    Csc "CSC" 1..=1 Math Implemented;
    Sec "SEC" 1..=1 Math Implemented;
    Cot "COT" 1..=1 Math Implemented;
    Sinh "SINH" 1..=1 Math Implemented;
    Cosh "COSH" 1..=1 Math Implemented;
    Tanh "TANH" 1..=1 Math Implemented;
    Asinh "ASINH" 1..=1 Math Implemented;
    Acosh "ACOSH" 1..=1 Math Implemented;
    Atanh "ATANH" 1..=1 Math Implemented;
    Floor "FLOOR" 1..=1 Math Implemented;
    Ceiling "CEILING" 1..=1 Math Implemented;
    Round "ROUND" 1..=2 Math Implemented;
    Truncate "TRUNCATE" 1..=2 Math Implemented;
    Ip "IP" 1..=1 Math Implemented;
    Fp "FP" 1..=1 Math Implemented;
    Sign "SIGN" 1..=1 Math Implemented;
    Max "MAX" 1..=N Math Implemented;
    Min "MIN" 1..=N Math Implemented;
    Comb "COMB" 2..=2 Math Implemented;
    Perm "PERM" 2..=2 Math Implemented;
    Random "RANDOM" 0..=3 Math Implemented;
    Randint "RANDINT" 0..=3 Math Implemented;
    Randnorm "RANDNORM" 0..=2 Math Implemented;
    Randseed "RANDSEED" 0..=1 Math Implemented;
    Mant "MANT" 1..=1 Math Implemented;
    Xpon "XPON" 1..=1 Math Implemented;
    Re "RE" 1..=1 Math Implemented;
    Im "IM" 1..=1 Math Implemented;
    Conj "CONJ" 1..=1 Math Implemented;
    Arg "ARG" 1..=1 Math Implemented;
    Approx "APPROX" 1..=1 Math Approximate;
    Exact "EXACT" 1..=1 Math Unsupported;
    Hms "→HMS" 1..=1 Math Implemented;
    HmsFrom "HMS→" 1..=1 Math Implemented;
    Nthroot "NTHROOT" 2..=2 Math Implemented;
    Factorial "FACTORIAL" 1..=1 Math Implemented;
    Mean "MEAN" 1..=2 Math Implemented;
    Median "MEDIAN" 1..=2 Math Implemented;
    Stddev "STDDEV" 1..=2 Math Approximate;
    Ifte "IFTE" 3..=3 Math Implemented;
    // ---- integers / bits ----
    Bitand "BITAND" 2..=N Bits Implemented;
    Bitor "BITOR" 2..=N Bits Implemented;
    Bitxor "BITXOR" 2..=N Bits Implemented;
    Bitnot "BITNOT" 1..=1 Bits Implemented;
    Bitsl "BITSL" 1..=2 Bits Implemented;
    Bitsr "BITSR" 1..=2 Bits Implemented;
    RtoB "R→B" 1..=1 Bits Implemented;
    BtoR "B→R" 1..=1 Bits Implemented;
    Setbase "SETBASE" 1..=2 Bits Implemented;
    Getbase "GETBASE" 1..=1 Bits Implemented;
    Setbits "SETBITS" 1..=2 Bits Implemented;
    Getbits "GETBITS" 1..=1 Bits Implemented;
    // ---- strings ----
    Asc "ASC" 1..=1 Strings Implemented;
    Char "CHAR" 1..=1 Strings Implemented;
    Dim "DIM" 1..=1 Strings Implemented;
    Instring "INSTRING" 2..=2 Strings Implemented;
    Left "LEFT" 2..=2 Strings Implemented;
    Right "RIGHT" 2..=2 Strings Implemented;
    Mid "MID" 2..=3 Strings Implemented;
    Rotate "ROTATE" 1..=2 Strings Implemented;
    StringFn "STRING" 1..=5 Strings Approximate;
    Upper "UPPER" 1..=1 Strings Implemented;
    Lower "LOWER" 1..=1 Strings Implemented;
    Replace "REPLACE" 3..=3 Strings Implemented;
    Expr "EXPR" 1..=1 Strings Approximate;
    Concat "CONCAT" 2..=N Lists Implemented;
    // ---- lists ----
    Makelist "MAKELIST" 2..=5 Lists Implemented;
    Size "SIZE" 1..=1 Lists Implemented;
    Sort "SORT" 1..=2 Lists Implemented;
    Reverse "REVERSE" 1..=1 Lists Implemented;
    Pos "POS" 2..=2 Lists Implemented;
    SumList "ΣLIST" 1..=1 Lists Implemented;
    ProdList "∏LIST" 1..=1 Lists Implemented;
    DeltaList "ΔLIST" 1..=1 Lists Implemented;
    Equal "EQ" 2..=2 Lists Implemented;
    Head "HEAD" 1..=1 Lists Implemented;
    Tail "TAIL" 1..=1 Lists Implemented;
    Append "APPEND" 2..=2 Lists Approximate;
    // ---- matrices ----
    Makemat "MAKEMAT" 2..=3 Matrices Implemented;
    Det "DET" 1..=1 Matrices Implemented;
    Trn "TRN" 1..=1 Matrices Implemented;
    Idenmat "IDENMAT" 1..=1 Matrices Implemented;
    Rank "RANK" 1..=1 Matrices Implemented;
    Colnorm "COLNORM" 1..=1 Matrices Implemented;
    Rownorm "ROWNORM" 1..=1 Matrices Implemented;
    Inverse "INVERSE" 1..=1 Matrices Implemented;
    Addrow "ADDROW" 3..=3 Matrices Implemented;
    Addcol "ADDCOL" 3..=3 Matrices Implemented;
    Delrow "DELROW" 2..=2 Matrices Implemented;
    Delcol "DELCOL" 2..=2 Matrices Implemented;
    Redim "REDIM" 2..=2 Matrices Implemented;
    Dot "DOT" 2..=2 Matrices Implemented;
    Cross "CROSS" 2..=2 Matrices Implemented;
    Editmat "EDITMAT" 1..=3 Matrices Unsupported;
    // ---- graphics ----
    RectP "RECT_P" 0..=7 Graphics Implemented;
    Rect "RECT" 0..=7 Graphics Approximate;
    LineP "LINE_P" 2..=6 Graphics Implemented;
    Line "LINE" 2..=6 Graphics Approximate;
    PixonP "PIXON_P" 2..=4 Graphics Implemented;
    Pixon "PIXON" 2..=4 Graphics Approximate;
    PixoffP "PIXOFF_P" 2..=3 Graphics Implemented;
    Pixoff "PIXOFF" 2..=3 Graphics Approximate;
    GetpixP "GETPIX_P" 2..=3 Graphics Implemented;
    Getpix "GETPIX" 2..=3 Graphics Approximate;
    CircleP "CIRCLE_P" 3..=5 Graphics Implemented;
    Circle "CIRCLE" 3..=5 Graphics Approximate;
    FillcircleP "FILLCIRCLE_P" 3..=5 Graphics Approximate;
    ArcP "ARC_P" 3..=7 Graphics Implemented;
    Arc "ARC" 3..=7 Graphics Approximate;
    TextoutP "TEXTOUT_P" 3..=8 Graphics Approximate;
    Textout "TEXTOUT" 3..=8 Graphics Approximate;
    BlitP "BLIT_P" 0..=11 Graphics Implemented;
    Blit "BLIT" 0..=11 Graphics Approximate;
    DimgrobP "DIMGROB_P" 2..=4 Graphics Implemented;
    Dimgrob "DIMGROB" 2..=4 Graphics Approximate;
    SubgrobP "SUBGROB_P" 2..=6 Graphics Implemented;
    Subgrob "SUBGROB" 2..=6 Graphics Approximate;
    InvertP "INVERT_P" 0..=5 Graphics Implemented;
    FillpolyP "FILLPOLY_P" 2..=4 Graphics Implemented;
    TriangleP "TRIANGLE_P" 0..=15 Graphics Approximate;
    GrobwP "GROBW_P" 1..=1 Graphics Implemented;
    GrobhP "GROBH_P" 1..=1 Graphics Implemented;
    Rgb "RGB" 3..=4 Graphics Implemented;
    // ---- input / output ----
    Print "PRINT" 0..=1 Io Implemented;
    Msgbox "MSGBOX" 1..=2 Io Approximate;
    Input "INPUT" 1..=6 Io Approximate ref 1;
    Choose "CHOOSE" 2..=N Io Approximate ref 1;
    Getkey "GETKEY" 0..=0 Io Implemented;
    Iskeydown "ISKEYDOWN" 1..=1 Io Implemented;
    Wait "WAIT" 0..=1 Io Implemented;
    Mouse "MOUSE" 0..=1 Io Approximate;
    Disp "DISP" 2..=2 Io Approximate;
    Freeze "FREEZE" 0..=0 Io Approximate;
    Drawmenu "DRAWMENU" 0..=6 Io Approximate;
    Editlist "EDITLIST" 1..=1 Io Unsupported;
    // ---- time ----
    Ticks "TICKS" 0..=0 Time Implemented;
    Time "TIME" 0..=0 Time Implemented;
    Date "DATE" 0..=0 Time Implemented;
    // ---- system ----
    Type "TYPE" 1..=1 System Implemented;
    Eval "EVAL" 1..=1 System Unsupported;
    Startapp "STARTAPP" 1..=1 System Unsupported;
    Startview "STARTVIEW" 1..=2 System Unsupported;
    Recall "RECALL" 1..=1 System Unsupported;
    Notes "NOTES" 0..=1 System Implemented;
    Programs "PROGRAMS" 0..=1 System Implemented;
    Afiles "AFILES" 0..=1 System Unsupported;
    Hvars "HVARS" 0..=1 System Unsupported;
    // ---- CAS (separate subsystem, not emulated) ----
    CasCall "CAS" 0..=N Cas Unsupported;
}

pub fn spec(b: Builtin) -> &'static BuiltinSpec {
    &BUILTINS[b as usize]
}

/// Name lookup. Command names are upper case on the Prime; a lower-case
/// spelling resolves too but the caller should warn, because on the
/// calculator lower-case names call the CAS version.
pub fn lookup(name: &str) -> Option<&'static BuiltinSpec> {
    // Hashed: editors call this for every identifier on every keystroke.
    static INDEX: std::sync::OnceLock<std::collections::HashMap<&'static str, usize>> =
        std::sync::OnceLock::new();
    let index = INDEX.get_or_init(|| {
        BUILTINS
            .iter()
            .enumerate()
            .map(|(i, s)| (s.name, i))
            .collect()
    });
    if let Some(&i) = index.get(name) {
        return Some(&BUILTINS[i]);
    }
    let alias = match name {
        "√" => "SQRT",
        "CHR" => "CHAR",
        "SIGMALIST" => "ΣLIST",
        "PILIST" => "∏LIST",
        "DELTALIST" => "ΔLIST",
        "R->B" | "R_to_B" => "R→B",
        "B->R" | "B_to_R" => "B→R",
        "INV" => "INVERSE",
        _ => "",
    };
    index.get(alias).map(|&i| &BUILTINS[i])
}

pub fn lookup_case_insensitive(name: &str) -> Option<&'static BuiltinSpec> {
    let up = name.to_uppercase();
    lookup(&up)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_consistent() {
        for (i, s) in BUILTINS.iter().enumerate() {
            assert_eq!(s.builtin as usize, i, "{} out of order", s.name);
            assert!(s.max == VARIADIC || s.min <= s.max, "{}", s.name);
        }
        let mut names: Vec<_> = BUILTINS.iter().map(|s| s.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), BUILTINS.len());
    }

    #[test]
    fn lookups() {
        assert_eq!(lookup("RECT_P").unwrap().builtin, Builtin::RectP);
        assert_eq!(lookup("B→R").unwrap().builtin, Builtin::BtoR);
        assert!(lookup("rect_p").is_none());
        assert!(lookup_case_insensitive("rect_p").is_some());
        assert!(lookup("RECT_P").unwrap().accepts(5));
    }
}
