//! Expression evaluation — (eval_op, matching_quote,
//! matching_paren, eval_expr, eval_lhs_op_rhs, process_if) with the exact
//! token scan and first-op-wins right-associative split.

use super::variables::{c_is_number, str_str, trig_ident, var_subst};
use super::{atoi32, script_log, DgCtx};
use crate::game::Game;
use mud_data::types::MAX_INPUT_LENGTH;

pub type BStr = Vec<u8>;

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

/// Trim ASCII whitespace from both ends (eval_op's in-place trims).
fn trim(s: &[u8]) -> &[u8] {
    let start = s.iter().position(|&b| !is_ws(b)).unwrap_or(s.len());
    let s = &s[start..];
    let end = s.iter().rposition(|&b| !is_ws(b)).map_or(0, |e| e + 1);
    &s[..end]
}

pub fn eval_op(op: &[u8], lhs: &[u8], rhs: &[u8]) -> BStr {
    let lhs = trim(lhs);
    let rhs = trim(rhs);
    let num = |b: &[u8]| atoi32(b);
    let truthy = |b: &[u8]| !b.is_empty() && b[0] != b'0';

    let s = |v: i32| v.to_string().into_bytes();
    match op {
        b"||" => {
            if !truthy(lhs) && !truthy(rhs) {
                b"0".to_vec()
            } else {
                b"1".to_vec()
            }
        }
        b"&&" => {
            if !truthy(lhs) || !truthy(rhs) {
                b"0".to_vec()
            } else {
                b"1".to_vec()
            }
        }
        b"==" => {
            if c_is_number(lhs) && c_is_number(rhs) {
                s((num(lhs) == num(rhs)) as i32)
            } else {
                s(lhs.eq_ignore_ascii_case(rhs) as i32)
            }
        }
        b"!=" => {
            if c_is_number(lhs) && c_is_number(rhs) {
                s((num(lhs) != num(rhs)) as i32)
            } else {
                s(!lhs.eq_ignore_ascii_case(rhs) as i32)
            }
        }
        b"<=" => {
            if c_is_number(lhs) && c_is_number(rhs) {
                s((num(lhs) <= num(rhs)) as i32)
            } else {
                s((cmp_ci(lhs, rhs) <= 0) as i32)
            }
        }
        b">=" => {
            if c_is_number(lhs) && c_is_number(rhs) {
                s((num(lhs) >= num(rhs)) as i32)
            } else {
                // A quirk kept deliberately: string >= computes
                // Case-insensitive, less-or-equal.
                s((cmp_ci(lhs, rhs) <= 0) as i32)
            }
        }
        b"<" => {
            if c_is_number(lhs) && c_is_number(rhs) {
                s((num(lhs) < num(rhs)) as i32)
            } else {
                s((cmp_ci(lhs, rhs) < 0) as i32)
            }
        }
        b">" => {
            if c_is_number(lhs) && c_is_number(rhs) {
                s((num(lhs) > num(rhs)) as i32)
            } else {
                s((cmp_ci(lhs, rhs) > 0) as i32)
            }
        }
        b"/=" => {
            if str_str(lhs, rhs) { b"1".to_vec() } else { b"0".to_vec() }
        }
        b"*" => s(num(lhs).wrapping_mul(num(rhs))),
        b"/" => {
            let n = num(rhs);
            s(if n != 0 { num(lhs).wrapping_div(n) } else { 0 })
        }
        b"+" => s(num(lhs).wrapping_add(num(rhs))),
        b"-" => s(num(lhs).wrapping_sub(num(rhs))),
        b"!" => {
            if c_is_number(rhs) {
                s((num(rhs) == 0) as i32)
            } else {
                s(rhs.is_empty() as i32)
            }
        }
        _ => Vec::new(),
    }
}

/// Case-insensitive byte ordering, returning <0, 0 or >0.
fn cmp_ci(a: &[u8], b: &[u8]) -> i32 {
    let mut i = 0;
    loop {
        let ca = a.get(i).copied().unwrap_or(0).to_ascii_lowercase();
        let cb = b.get(i).copied().unwrap_or(0).to_ascii_lowercase();
        if ca != cb {
            return ca as i32 - cb as i32;
        }
        if ca == 0 {
            return 0;
        }
        i += 1;
    }
}

/// matching_quote: index of the closing quote (or last
/// char). `p` = index of the opening quote.
pub fn matching_quote(s: &[u8], p: usize) -> usize {
    let mut i = p + 1;
    while i < s.len() && s[i] != b'"' {
        if s[i] == b'\\' {
            i += 1;
        }
        i += 1;
    }
    if i >= s.len() {
        i = s.len().saturating_sub(1);
    }
    i
}

/// matching_paren: index of the matching ')' (or last
/// char before the terminator).
pub fn matching_paren(s: &[u8], p: usize) -> usize {
    let mut i = p + 1;
    let mut depth = 1;
    while i < s.len() && depth != 0 {
        match s[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b'"' => i = matching_quote(s, i),
            _ => {}
        }
        i += 1;
    }
    i.saturating_sub(1)
}

pub fn eval_expr(g: &mut Game, ctx: DgCtx, line: &[u8]) -> BStr {
    let start = line.iter().position(|&b| !is_ws(b)).unwrap_or(line.len());
    let line = &line[start..];

    // B80, the other half. An if/while/switch condition reaches
    // eval_lhs_op_rhs without passing through var_subst at all, so both
    // entry points need the guard. An empty result reads as false in
    // process_if.
    if line.len() >= MAX_INPUT_LENGTH {
        let (name, tvnum) = trig_ident(g, ctx);
        script_log(
            g,
            &format!(
                "Trigger: {}, VNum {}, type: {}. Expression is {} characters, over the {} limit: '{}...'",
                name,
                tvnum,
                ctx.go.kind(),
                line.len(),
                MAX_INPUT_LENGTH - 1,
                String::from_utf8_lossy(&line[..60.min(line.len())])
            ),
        );
        return Vec::new();
    }

    if let Some(res) = eval_lhs_op_rhs(g, ctx, line) {
        res
    } else if line.first() == Some(&b'(') {
        let p = matching_paren(line, 0);
        // *p = '\0'; eval(expr + 1). If unbalanced, p = last char and the
        // closing byte is dropped either way.
        let inner = if p > 0 { &line[1..p] } else { &line[1..] };
        eval_expr(g, ctx, &inner.to_vec())
    } else {
        var_subst(g, ctx, line)
    }
}

/// eval_lhs_op_rhs. None = no operator found.
pub fn eval_lhs_op_rhs(g: &mut Game, ctx: DgCtx, expr: &[u8]) -> Option<BStr> {
    const OPS: [&[u8]; 14] = [
        b"||", b"&&", b"==", b"!=", b"<=", b">=", b"<", b">", b"/=", b"-", b"+", b"/", b"*", b"!",
    ];

    //

    // Token positions: skip (...) groups, "..." strings, and alnum+space runs.
    let mut tokens: Vec<usize> = Vec::new();
    let mut p = 0usize;
    while p < expr.len() {
        tokens.push(p);
        let c = expr[p];
        if c == b'(' {
            p = matching_paren(expr, p) + 1;
        } else if c == b'"' {
            p = matching_quote(expr, p) + 1;
        } else if c.is_ascii_alphanumeric() {
            p += 1;
            while p < expr.len() && (expr[p].is_ascii_alphanumeric() || is_ws(expr[p])) {
                p += 1;
            }
        } else {
            p += 1;
        }
    }

    for op in OPS {
        for &tok in &tokens {
            if tok + op.len() <= expr.len()
                && expr[tok..tok + op.len()].eq_ignore_ascii_case(op)
            {
                let lhs = &expr[..tok];
                let rhs = &expr[tok + op.len()..];
                let lhr = eval_expr(g, ctx, &lhs.to_vec());
                let rhr = eval_expr(g, ctx, &rhs.to_vec());
                return Some(eval_op(op, &lhr, &rhr));
            }
        }
    }
    None
}

/// process_if: truthy = non-empty and first non-space
/// char != '0'.
pub fn process_if(g: &mut Game, ctx: DgCtx, cond: &[u8]) -> bool {
    let result = eval_expr(g, ctx, cond);
    let p = result.iter().position(|&b| !is_ws(b));
    match p {
        None => false,
        Some(i) => result[i] != b'0',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ops() {
        assert_eq!(eval_op(b"==", b" 4 ", b"4"), b"1");
        assert_eq!(eval_op(b"==", b"abc", b"ABC"), b"1");
        assert_eq!(eval_op(b">=", b"b", b"a"), b"0"); // string >= is <= (bug)
        assert_eq!(eval_op(b"<=", b"a", b"b"), b"1");
        assert_eq!(eval_op(b"/=", b"yes", b"ye"), b"1");
        assert_eq!(eval_op(b"/=", b"yes", b""), b"0");
        assert_eq!(eval_op(b"/", b"7", b"0"), b"0");
        assert_eq!(eval_op(b"!", b"", b"0"), b"1");
        assert_eq!(eval_op(b"!", b"", b"hi"), b"0");
        assert_eq!(eval_op(b"||", b"01", b""), b"0"); // "01" is falsy
        assert_eq!(eval_op(b"&&", b"1", b"x"), b"1");
    }

    #[test]
    fn matching_helpers() {
        assert_eq!(matching_paren(b"(a(b)c)d", 0), 6);
        assert_eq!(matching_paren(b"(a\"()\"b)", 0), 7);
        assert_eq!(matching_quote(b"\"a\\\"b\"c", 0), 5);
        // Unbalanced: last char.
        assert_eq!(matching_paren(b"(abc", 0), 3);
    }
}
