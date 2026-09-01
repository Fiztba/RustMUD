//! Socials: the socials.new parser (boot_social_messages) and the
//! command-list merge (create_command_list). do_action lives in
//! act/social command code.

use std::path::Path;

use crate::text::parse_at;

pub type BStr = Vec<u8>;

/// struct social_messg.
#[derive(Debug, Clone, Default)]
pub struct Social {
    /// Runtime index in the merged command list (backpatched on merge).
    pub act_nr: usize,
    pub command: BStr,
    pub sort_as: BStr,
    pub hide: i32,
    pub min_char_position: i32,
    pub min_victim_position: i32,
    pub min_level_char: i32,
    pub char_no_arg: Option<BStr>,
    pub others_no_arg: Option<BStr>,
    pub char_found: Option<BStr>,
    pub others_found: Option<BStr>,
    pub vict_found: Option<BStr>,
    pub not_found: Option<BStr>,
    pub char_auto: Option<BStr>,
    pub others_auto: Option<BStr>,
    pub char_body_found: Option<BStr>,
    pub others_body_found: Option<BStr>,
    pub vict_body_found: Option<BStr>,
    pub char_obj_found: Option<BStr>,
    pub others_obj_found: Option<BStr>,
}

/// fread_action: one line; '#' → None; parse_at; cut at CR/LF.
struct Lines<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Lines<'a> {
    fn next_line(&mut self) -> Option<&'a [u8]> {
        if self.pos >= self.data.len() {
            return None;
        }
        let start = self.pos;
        while self.pos < self.data.len() && self.data[self.pos] != b'\n' {
            self.pos += 1;
        }
        let mut end = self.pos;
        if self.pos < self.data.len() {
            self.pos += 1; // consume the \n
        }
        if end > start && self.data[end - 1] == b'\r' {
            end -= 1;
        }
        Some(&self.data[start..end])
    }

    /// Skip whitespace, take a word, then skip the whitespace after it.
    fn next_word(&mut self) -> Option<BStr> {
        while self.pos < self.data.len() && self.data[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
        if self.pos >= self.data.len() {
            return None;
        }
        let start = self.pos;
        while self.pos < self.data.len() && !self.data[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
        let w = self.data[start..self.pos].to_vec();
        while self.pos < self.data.len() && self.data[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
        Some(w)
    }
}

fn fread_action(lines: &mut Lines) -> Option<BStr> {
    let line = lines.next_line()?;
    if line.first() == Some(&b'#') {
        return None;
    }
    let mut b = line.to_vec();
    parse_at(&mut b);
    Some(b)
}

/// boot_social_messages for the new format (CONFIG_NEW_SOCIALS, the
/// default). Returns (socials, log lines).
pub fn boot_social_messages(lib: &Path) -> Result<(Vec<Social>, Vec<String>), String> {
    let path = lib.join("misc").join("socials.new");
    let data = std::fs::read(&path)
        .map_err(|e| format!("SYSERR: can't open socials file '{}': {}", path.display(), e))?;
    let mut log = Vec::new();
    let count = data.split(|c| *c == b'\n').filter(|l| l.first() == Some(&b'~')).count();
    log.push(format!("Social table contains {} socials.", count));

    let mut socials = Vec::with_capacity(count);
    let mut lines = Lines { data: &data, pos: 0 };
    loop {
        let Some(next_soc) = lines.next_word() else {
            return Err("SYSERR: unexpected end of file encountered in socials file".to_string());
        };
        if next_soc.first() == Some(&b'$') {
            break;
        }
        // " %s %d %d %d %d \n"
        let sorted = lines.next_word().ok_or("SYSERR: format error in social file")?;
        let hide = lines.next_word().ok_or("format")?;
        let min_char_pos = lines.next_word().ok_or("format")?;
        let min_pos = lines.next_word().ok_or("format")?;
        let min_lvl = lines.next_word().ok_or("format")?;
        // The trailing-whitespace skip already consumed the newline; the
        // action lines start at the current position.
        let mut s = Social {
            command: next_soc.get(1..).unwrap_or(b"").to_vec(),
            sort_as: sorted,
            hide: crate::handler::atoi(&hide),
            min_char_position: crate::handler::atoi(&min_char_pos),
            min_victim_position: crate::handler::atoi(&min_pos),
            min_level_char: crate::handler::atoi(&min_lvl),
            ..Default::default()
        };
        s.char_no_arg = fread_action(&mut lines);
        s.others_no_arg = fread_action(&mut lines);
        s.char_found = fread_action(&mut lines);
        s.others_found = fread_action(&mut lines);
        s.vict_found = fread_action(&mut lines);
        s.not_found = fread_action(&mut lines);
        s.char_auto = fread_action(&mut lines);
        s.others_auto = fread_action(&mut lines);
        s.char_body_found = fread_action(&mut lines);
        s.others_body_found = fread_action(&mut lines);
        s.vict_body_found = fread_action(&mut lines);
        s.char_obj_found = fread_action(&mut lines);
        s.others_obj_found = fread_action(&mut lines);
        socials.push(s);
    }
    Ok((socials, log))
}

/// The selection sort from create_command_list (exact algorithm — its tie
/// behavior is part of the merge order).
pub fn sort_socials(socials: &mut [Social]) {
    let n = socials.len();
    if n == 0 {
        return;
    }
    for j in 0..n - 1 {
        let mut k = j;
        for i in j + 1..n {
            if crate::text::cmp_ci(&socials[i].sort_as, &socials[k].sort_as) == std::cmp::Ordering::Less {
                k = i;
            }
        }
        if j != k {
            socials.swap(j, k);
        }
    }
}
