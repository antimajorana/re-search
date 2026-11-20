//! Deterministic expression sanity & numerical spot-check tool (`check_expression_sanity`).

use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tools::{ApprovalScope, CallCtx, Registry, ToolDef};

pub const CHECK_EXPRESSION_SANITY: &str = "check_expression_sanity";

const SAMPLE_BASES: [f64; 5] = [
    0.731_928_41,
    std::f64::consts::SQRT_2,
    2.236_067_98,
    std::f64::consts::PI,
    0.371_828_18,
];

pub fn check_expression_sanity_def() -> ToolDef {
    use policy::EffectClass;
    ToolDef {
        id: CHECK_EXPRESSION_SANITY.into(),
        description: "Check mathematical equality, limits, or dimensional/bracket balance between lhs and rhs.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "lhs": { "type": "string", "maxLength": 4000 },
                "rhs": { "type": "string", "maxLength": 4000 },
                "variables": { "type": "array", "items": { "type": "string" } },
                "domain": { "type": "string", "enum": ["real", "positive", "complex"] }
            },
            "required": ["lhs", "rhs"],
            "additionalProperties": false
        }),
        effect: EffectClass::ObserveLocal,
        approval_scope: ApprovalScope::Once,
    }
}

pub fn register(registry: &mut Registry) {
    registry.register(
        check_expression_sanity_def(),
        Arc::new(|_ctx: &CallCtx<'_>, args: &Value| run_expression_sanity(args)),
    );
}

pub fn run_expression_sanity(args: &Value) -> Result<Value, String> {
    let lhs = args.get("lhs").and_then(Value::as_str).ok_or("`lhs` must be a string")?.trim();
    let rhs = args.get("rhs").and_then(Value::as_str).ok_or("`rhs` must be a string")?.trim();
    if lhs.is_empty() || rhs.is_empty() {
        return Err("`lhs` and `rhs` must not be empty".into());
    }
    let domain = args.get("domain").and_then(Value::as_str).unwrap_or("real");
    if !matches!(domain, "real" | "positive" | "complex") {
        return Err(format!("unsupported domain `{domain}`"));
    }
    let explicit_vars: Vec<String> = match args.get("variables") {
        None => Vec::new(),
        Some(v) => v
            .as_array()
            .ok_or("`variables` must be an array of strings")?
            .iter()
            .map(|x| x.as_str().map(str::trim).map(str::to_string).ok_or("variable names must be strings"))
            .collect::<Result<Vec<_>, _>>()?,
    };

    let lhs_bal = check_delimiters(lhs);
    let rhs_bal = check_delimiters(rhs);
    let delimiter_balanced = lhs_bal.is_none() && rhs_bal.is_none();

    let mut issues = Vec::new();
    if let Some(m) = lhs_bal {
        issues.push(format!("lhs: {m}"));
    }
    if let Some(m) = rhs_bal {
        issues.push(format!("rhs: {m}"));
    }

    let lhs_ast = parse_expr(lhs);
    let rhs_ast = parse_expr(rhs);
    let mut numeric_eval_supported = false;
    let mut numeric_match = None;
    let mut max_abs_diff: Option<f64> = None;
    let mut samples_checked = 0usize;

    if delimiter_balanced {
        if let (Ok(l_node), Ok(r_node)) = (&lhs_ast, &rhs_ast) {
            let mut vars = BTreeSet::new();
            collect_vars(l_node, &mut vars);
            collect_vars(r_node, &mut vars);
            for v in &explicit_vars {
                if !v.is_empty() {
                    vars.insert(v.clone());
                }
            }
            let var_list: Vec<String> = vars.into_iter().collect();
            let mut max_diff = 0.0f64;
            let mut all_ok = true;
            let mut evaluated_any = false;
            for (sample_idx, &base) in SAMPLE_BASES.iter().enumerate() {
                let mut env = BTreeMap::new();
                for (vi, vname) in var_list.iter().enumerate() {
                    let shift = 1.0 + 0.193 * (vi as f64) + 0.071 * (sample_idx as f64);
                    let val = if domain == "positive" {
                        (base * shift).abs() + 0.25
                    } else if sample_idx % 2 == 1 {
                        -(base * shift)
                    } else {
                        base * shift
                    };
                    env.insert(vname.clone(), val);
                }
                match (eval_node(l_node, &env), eval_node(r_node, &env)) {
                    (Ok(lv), Ok(rv)) if lv.is_finite() && rv.is_finite() => {
                        evaluated_any = true;
                        samples_checked += 1;
                        let diff = (lv - rv).abs();
                        if diff > max_diff {
                            max_diff = diff;
                        }
                        let scale = lv.abs().max(rv.abs()).max(1.0);
                        if diff > 1e-9 * scale {
                            all_ok = false;
                        }
                    }
                    _ => {}
                }
            }
            if evaluated_any {
                numeric_eval_supported = true;
                numeric_match = Some(all_ok);
                max_abs_diff = Some(max_diff);
                if !all_ok {
                    issues.push(format!("numeric mismatch across {samples_checked} samples (max_abs_diff={max_diff:.3e})"));
                }
            }
        }
    }

    let ok = delimiter_balanced && numeric_match.unwrap_or(true) && issues.is_empty();
    Ok(json!({
        "ok": ok,
        "lhs": lhs,
        "rhs": rhs,
        "domain": domain,
        "delimiter_balanced": delimiter_balanced,
        "numeric_eval_supported": numeric_eval_supported,
        "numeric_match": numeric_match,
        "samples_checked": samples_checked,
        "max_abs_diff": max_abs_diff,
        "issues": issues,
    }))
}

fn check_delimiters(s: &str) -> Option<String> {
    let mut stack: Vec<char> = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        match chars[i] {
            '(' | '[' | '{' => stack.push(chars[i]),
            ')' | ']' | '}' => {
                let expected = match chars[i] {
                    ')' => '(',
                    ']' => '[',
                    _ => '{',
                };
                match stack.pop() {
                    Some(top) if top == expected => {}
                    Some(top) => return Some(format!("mismatched delimiter `{top}` and `{}`", chars[i])),
                    None => return Some(format!("unexpected closing `{}`", chars[i])),
                }
            }
            _ => {}
        }
        i += 1;
    }
    stack.pop().map(|c| format!("unclosed delimiter `{c}`"))
}

#[derive(Debug, Clone)]
enum Node {
    Num(f64),
    Var(String),
    Neg(Box<Node>),
    BinOp(char, Box<Node>, Box<Node>),
    Call(String, Vec<Node>),
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Op(char),
    LParen,
    RParen,
    Comma,
}

fn tokenize(s: &str) -> Result<Vec<Tok>, String> {
    let normalized = s.replace("**", "^");
    let bytes = normalized.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b.is_ascii_digit() || (b == b'.' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit()) {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            if i < bytes.len() && matches!(bytes[i], b'e' | b'E') {
                i += 1;
                if i < bytes.len() && matches!(bytes[i], b'+' | b'-') {
                    i += 1;
                }
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let num: f64 = normalized[start..i].parse().map_err(|_| "invalid number")?;
            toks.push(Tok::Num(num));
            continue;
        }
        if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            toks.push(Tok::Ident(normalized[start..i].to_string()));
            continue;
        }
        match b {
            b'+' | b'-' | b'*' | b'/' | b'^' => toks.push(Tok::Op(b as char)),
            b'(' => toks.push(Tok::LParen),
            b')' => toks.push(Tok::RParen),
            b',' => toks.push(Tok::Comma),
            _ => return Err(format!("unsupported token `{}`", b as char)),
        }
        i += 1;
    }
    Ok(toks)
}

fn parse_expr(s: &str) -> Result<Node, String> {
    let toks = tokenize(s)?;
    let mut pos = 0usize;
    let node = parse_add(&toks, &mut pos)?;
    if pos != toks.len() {
        return Err("trailing tokens".into());
    }
    Ok(node)
}

fn parse_add(toks: &[Tok], pos: &mut usize) -> Result<Node, String> {
    let mut lhs = parse_mul(toks, pos)?;
    while let Some(Tok::Op(op @ ('+' | '-'))) = toks.get(*pos) {
        *pos += 1;
        let rhs = parse_mul(toks, pos)?;
        lhs = Node::BinOp(*op, Box::new(lhs), Box::new(rhs));
    }
    Ok(lhs)
}

fn parse_mul(toks: &[Tok], pos: &mut usize) -> Result<Node, String> {
    let mut lhs = parse_unary(toks, pos)?;
    while let Some(Tok::Op(op @ ('*' | '/'))) = toks.get(*pos) {
        *pos += 1;
        let rhs = parse_unary(toks, pos)?;
        lhs = Node::BinOp(*op, Box::new(lhs), Box::new(rhs));
    }
    Ok(lhs)
}

fn parse_unary(toks: &[Tok], pos: &mut usize) -> Result<Node, String> {
    if let Some(Tok::Op('-')) = toks.get(*pos) {
        *pos += 1;
        return Ok(Node::Neg(Box::new(parse_unary(toks, pos)?)));
    }
    if let Some(Tok::Op('+')) = toks.get(*pos) {
        *pos += 1;
        return parse_unary(toks, pos);
    }
    parse_pow(toks, pos)
}

fn parse_pow(toks: &[Tok], pos: &mut usize) -> Result<Node, String> {
    let base = parse_primary(toks, pos)?;
    if let Some(Tok::Op('^')) = toks.get(*pos) {
        *pos += 1;
        let exp = parse_unary(toks, pos)?;
        return Ok(Node::BinOp('^', Box::new(base), Box::new(exp)));
    }
    Ok(base)
}

fn parse_primary(toks: &[Tok], pos: &mut usize) -> Result<Node, String> {
    match toks.get(*pos) {
        Some(Tok::Num(n)) => {
            *pos += 1;
            Ok(Node::Num(*n))
        }
        Some(Tok::Ident(name)) => {
            *pos += 1;
            if matches!(toks.get(*pos), Some(Tok::LParen)) {
                *pos += 1;
                let mut args = Vec::new();
                if !matches!(toks.get(*pos), Some(Tok::RParen)) {
                    loop {
                        args.push(parse_add(toks, pos)?);
                        if matches!(toks.get(*pos), Some(Tok::Comma)) {
                            *pos += 1;
                        } else {
                            break;
                        }
                    }
                }
                if !matches!(toks.get(*pos), Some(Tok::RParen)) {
                    return Err("expected `)`".into());
                }
                *pos += 1;
                Ok(Node::Call(name.clone(), args))
            } else {
                Ok(Node::Var(name.clone()))
            }
        }
        Some(Tok::LParen) => {
            *pos += 1;
            let inner = parse_add(toks, pos)?;
            if !matches!(toks.get(*pos), Some(Tok::RParen)) {
                return Err("expected `)`".into());
            }
            *pos += 1;
            Ok(inner)
        }
        _ => Err("unexpected token".into()),
    }
}

fn collect_vars(node: &Node, out: &mut BTreeSet<String>) {
    match node {
        Node::Num(_) => {}
        Node::Var(v) => {
            if !matches!(v.as_str(), "pi" | "e") {
                out.insert(v.clone());
            }
        }
        Node::Neg(a) => collect_vars(a, out),
        Node::BinOp(_, a, b) => {
            collect_vars(a, out);
            collect_vars(b, out);
        }
        Node::Call(_, args) => {
            for a in args {
                collect_vars(a, out);
            }
        }
    }
}

fn eval_node(node: &Node, env: &BTreeMap<String, f64>) -> Result<f64, String> {
    match node {
        Node::Num(n) => Ok(*n),
        Node::Var(v) => match v.as_str() {
            "pi" => Ok(std::f64::consts::PI),
            "e" => Ok(std::f64::consts::E),
            _ => env.get(v).copied().ok_or_else(|| format!("unbound variable `{v}`")),
        },
        Node::Neg(a) => Ok(-eval_node(a, env)?),
        Node::BinOp(op, a, b) => {
            let av = eval_node(a, env)?;
            let bv = eval_node(b, env)?;
            Ok(match op {
                '+' => av + bv,
                '-' => av - bv,
                '*' => av * bv,
                '/' => av / bv,
                '^' => av.powf(bv),
                _ => return Err("unknown op".into()),
            })
        }
        Node::Call(name, args) => {
            if args.len() != 1 {
                return Err("expected 1 arg".into());
            }
            let x = eval_node(&args[0], env)?;
            Ok(match name.as_str() {
                "sin" => x.sin(),
                "cos" => x.cos(),
                "tan" => x.tan(),
                "exp" => x.exp(),
                "ln" | "log" => x.ln(),
                "sqrt" => x.sqrt(),
                "abs" => x.abs(),
                "sinh" => x.sinh(),
                "cosh" => x.cosh(),
                "tanh" => x.tanh(),
                _ => return Err(format!("unsupported fn `{name}`")),
            })
        }
    }
}
