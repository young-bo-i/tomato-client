//! Title/suggest-word filter — line-for-line port of the C# `TitleConvert`
//! pipeline (`KolToolWeb/TextHandler/`).
//!
//! Strategy: a list of 12 "converter" rules is tried in order. Each
//! converter pulls a candidate substring out of the input using a
//! different bracket / separator pattern. The first candidate that
//! survives the post-processing chain wins; if all fail, we return
//! `None`.
//!
//! Post-processing (per candidate):
//!   1. drop leading/trailing newlines
//!   2. reject if it contains `#`
//!   3. colon strip — split on `:` / `：`, keep the second segment
//!      (Douyin titles often look like `📖:扛炮修真#…` and the real
//!      book name is after the colon)
//!   4. strip every non-CJK character (`[^一-龥]+` → "")
//!   5. reject unless length is in the 4–8 CJK character range — the
//!      number-tomato platform refuses anything outside this band
//!      ("仅支持汉字,字数限制为4-8字" / "仅支持汉字及阿拉伯数字,字数
//!      限制为4-8字"). Pre-filtering here is much cheaper than burning
//!      API calls only to be rejected later.
//!   6. reject if any blacklist phrase matches in either direction
//!
//! Returns `None` when the original C# would have returned `""` — a
//! deliberate departure: storing NULL in Postgres lets downstream
//! queries `WHERE title_filtered IS NOT NULL` cleanly.

const MIN_CHARS: usize = 4;
const MAX_CHARS: usize = 8;

/// Phrases that disqualify a candidate. Match is bidirectional:
/// reject if the candidate contains the phrase OR the phrase contains
/// the candidate. The latter case is intentional in the C# code — it
/// catches single-character noise like "名" / "铭" being a one-char
/// candidate that's swallowed by the longer blacklist entries.
const BLACKLIST: &[&str] = &[
  "看全文",
  "巜",
  "》",
  "完结",
  "必看",
  "更完",
  "提示",
  "剧情",
  "抖音",
  "系列",
  "账号",
  "故事",
  "看了多少遍",
  "虚构",
  "一口气看完",
  "看完",
  "更到底",
  "宝藏小说",
  "小说",
  "识别失败",
  "书名",
  "铭",
  "名",
  "左下角",
  "本集",
  "完结版",
  "言言咸鱼",
  "全文",
  "分钟",
  "合集",
  "第集",
  "放心看",
];

/// Filter a raw Douyin title (or suggest-word) down to a usable book
/// name. Returns `None` if no rule produces a candidate that passes the
/// cleanup chain.
pub fn filter(input: &str) -> Option<String> {
  if input.trim().is_empty() {
    return None;
  }
  // Newline strip is done once up-front; the C# version did it inside
  // the loop but the result is identical.
  let context = input.replace('\n', "");
  if context.trim().is_empty() {
    return None;
  }

  let converters: &[fn(&str) -> Option<String>] = &[
    convert_const,
    convert_empty,
    convert_brackets_jp,
    convert_brackets_full_paren,
    convert_brackets_v2,
    convert_brackets_normal,
    convert_book,
    convert_book_v2,
    convert_little_book,
    convert_well,
    convert_tilde,
    convert_comma,
  ];

  for conv in converters {
    let inner = match conv(&context) {
      Some(s) if !s.is_empty() => s,
      _ => continue,
    };
    if inner.contains('#') {
      continue;
    }
    let cleaned = clean(&inner);
    if cleaned.is_empty() {
      continue;
    }
    if !check_length(&cleaned) {
      continue;
    }
    if is_blacklisted(&cleaned) {
      continue;
    }
    return Some(cleaned);
  }
  None
}

// --- cleanup helpers ----------------------------------------------------

fn clean(s: &str) -> String {
  let after_colon = colon_strip(s);
  remove_non_chinese(after_colon)
}

/// Split on `:` or `：`. If there are 2+ segments, return the second
/// (matches C# `titles[1]`). Otherwise return the original.
fn colon_strip(s: &str) -> &str {
  let mut iter = s.split(|c: char| c == ':' || c == '：');
  let first = iter.next();
  match (first, iter.next()) {
    (Some(_), Some(second)) => second,
    _ => s,
  }
}

fn remove_non_chinese(s: &str) -> String {
  s.chars().filter(|c| is_cjk(*c)).collect()
}

fn is_cjk(c: char) -> bool {
  let cp = c as u32;
  (0x4e00..=0x9fa5).contains(&cp)
}

fn contains_chinese(s: &str) -> bool {
  s.chars().any(is_cjk)
}

fn check_length(s: &str) -> bool {
  let n = s.chars().count();
  n >= MIN_CHARS && n <= MAX_CHARS
}

fn is_blacklisted(s: &str) -> bool {
  BLACKLIST.iter().any(|b| s.contains(b) || b.contains(s))
}

// --- converters ---------------------------------------------------------

fn convert_const(s: &str) -> Option<String> {
  if !contains_chinese(s) {
    return None;
  }
  if s.contains('#') || s.contains('<') || s.contains('>') {
    return None;
  }
  Some(s.to_string())
}

fn convert_empty(s: &str) -> Option<String> {
  let stripped: String = s.chars().filter(|c| *c != ' ').collect();
  if !contains_chinese(&stripped) {
    return None;
  }
  if stripped.contains('#') || stripped.contains('<') || stripped.contains('>') {
    return None;
  }
  Some(stripped)
}

fn extract_between(s: &str, open: char, close: char) -> Option<String> {
  let bs = s.find(open)?;
  let after = &s[bs + open.len_utf8()..];
  let be = after.find(close)?;
  Some(after[..be].to_string())
}

fn convert_brackets_jp(s: &str) -> Option<String> {
  extract_between(s, '『', '』')
}
fn convert_brackets_full_paren(s: &str) -> Option<String> {
  extract_between(s, '（', '）')
}
fn convert_brackets_v2(s: &str) -> Option<String> {
  extract_between(s, '【', '】')
}
fn convert_brackets_normal(s: &str) -> Option<String> {
  extract_between(s, '[', ']')
}
fn convert_book(s: &str) -> Option<String> {
  extract_between(s, '《', '》')
}
fn convert_book_v2(s: &str) -> Option<String> {
  extract_between(s, '巜', '》')
}
fn convert_little_book(s: &str) -> Option<String> {
  extract_between(s, '<', '>')
}

/// `#`-split: take the first segment. After splitting on `#`, no segment
/// itself contains `#`, so the C# `Where(t => !t.Contains("#"))` is a
/// no-op — kept here only as a faithful port note.
fn convert_well(s: &str) -> Option<String> {
  let first = s.split('#').next()?.to_string();
  if first.is_empty() {
    None
  } else {
    Some(first)
  }
}

/// `~`/`～` split: take the first segment, BUT only if there's at least
/// one separator (C# returns "" when the array has just one element —
/// i.e. no separator at all). This is the only converter that requires
/// the separator to be present.
fn convert_tilde(s: &str) -> Option<String> {
  let parts: Vec<&str> = s.split(|c: char| c == '~' || c == '～').collect();
  if parts.len() <= 1 {
    return None;
  }
  let first = parts[0];
  if first.is_empty() {
    None
  } else {
    Some(first.to_string())
  }
}

/// `,`/`，` split: take the first segment. Falls through `,` then `，`,
/// matching C# `OnlySymbol(',')` then `OnlySymbol('，')`. Note that the
/// C# code returns the input unchanged when there's no comma at all,
/// because `Split` returns `[input]` and `FirstOrDefault` is `input`.
fn convert_comma(s: &str) -> Option<String> {
  // Try ASCII comma first.
  if s.contains(',') {
    let head = s.split(',').next()?.to_string();
    if !head.is_empty() {
      return Some(head);
    }
  }
  // Fall through to full-width comma.
  if s.contains('，') {
    let head = s.split('，').next()?.to_string();
    if !head.is_empty() {
      return Some(head);
    }
  }
  // C# falls through to "no comma → return whole input via the no-op
  // split path". We do the same so this converter can still produce a
  // candidate when the prior 11 all failed.
  Some(s.to_string())
}

// --- tests --------------------------------------------------------------

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn book_brackets() {
    // C# sample: "《新月池爷》#现言小说#小说推文#小说推荐"
    assert_eq!(filter("《新月池爷》#现言小说#小说推文#小说推荐"), Some("新月池爷".into()));
  }

  #[test]
  fn jp_brackets() {
    // C# sample: "#知乎好文推荐 📖『春晖牡丹』#…"
    assert_eq!(
      filter("#知乎好文推荐 📖『春晖牡丹』#适合女生看的小说推荐 #评分9.5分以上的小说"),
      Some("春晖牡丹".into())
    );
  }

  #[test]
  fn full_width_paren() {
    // C# sample: "（四海默认）zhi 呼呼呼 厚须#宝藏小说 #文荒推荐"
    assert_eq!(filter("（四海默认）zhi 呼呼呼 厚须#宝藏小说 #文荒推荐"), Some("四海默认".into()));
  }

  #[test]
  fn v2_brackets() {
    // C# sample: "【梦回如烟】领航版#漫画解说"
    assert_eq!(filter("【梦回如烟】领航版#漫画解说"), Some("梦回如烟".into()));
  }

  #[test]
  fn normal_brackets() {
    // C# sample: "抖音搜小程序[竹节文学]，输入[c91836]看全文"
    assert_eq!(filter("抖音搜小程序[竹节文学]，输入[c91836]看全文"), Some("竹节文学".into()));
  }

  #[test]
  fn space_padded() {
    // C# sample: "半 糖 往 事" — TitleEmpty strips spaces.
    assert_eq!(filter("半 糖 往 事"), Some("半糖往事".into()));
  }

  #[test]
  fn well_split() {
    // C# sample: "冷 冷 指 定 #小说推荐 #文荒推荐 @DOU+小助手 @抖音小助手"
    // TitleEmpty hits first (strips spaces) → contains `#` → skip.
    // Const hits with `#` → skip.
    // Empty hits with `#` after space-strip → skip.
    // … brackets skip … Well splits on '#' takes first "冷 冷 指 定 ".
    // After Clear: "冷冷指定" (4 chars, all CJK, not blacklisted).
    assert_eq!(filter("冷 冷 指 定 #小说推荐 #文荒推荐 @DOU+小助手 @抖音小助手"), Some("冷冷指定".into()));
  }

  #[test]
  fn tilde_split() {
    // C# sample: "暴走心声~~~ #给瓦火"
    assert_eq!(filter("暴走心声~~~ #给瓦火"), Some("暴走心声".into()));
  }

  #[test]
  fn colon_strip() {
    // C# sample: "📖:扛炮修真#知乎推文 #修仙推文 …"
    // Const fails (contains #). Empty fails. Brackets fail.
    // Well splits → first part "📖:扛炮修真". Clean: colon-strip → "扛炮修真".
    // 4 chars, OK.
    assert_eq!(filter("📖:扛炮修真#知乎推文 #修仙推文 #宝藏小说文荒推荐"), Some("扛炮修真".into()));
  }

  #[test]
  fn too_long() {
    // 9-char title with no separators → all converters produce the
    // whole string, fails CheckLength.
    assert_eq!(filter("一二三四五六七八九"), None);
  }

  #[test]
  fn too_short() {
    // Platform refuses < 4 chars. These were real failures from
    // production: 馆长 (2), 女特警 (3), 补拙记 (3).
    assert_eq!(filter("馆长"), None);
    assert_eq!(filter("女特警"), None);
    assert_eq!(filter("《补拙记》"), None);
  }

  #[test]
  fn min_length_boundary() {
    // 4 chars is the minimum the platform accepts.
    assert_eq!(filter("一二三四"), Some("一二三四".into()));
  }

  #[test]
  fn blacklisted() {
    // "宝藏小说" itself is in the blacklist (and "小说" too) → reject.
    assert_eq!(filter("《宝藏小说》"), None);
  }

  #[test]
  fn empty() {
    assert_eq!(filter(""), None);
    assert_eq!(filter("   "), None);
    assert_eq!(filter("\n\n\n"), None);
  }

  #[test]
  fn no_chinese() {
    assert_eq!(filter("hello world 12345"), None);
  }

  #[test]
  fn suggest_word_typical() {
    // Real Douyin suggest-word from the dump — 7 CJK, no decoration.
    assert_eq!(filter("乡野田间的视频"), Some("乡野田间的视频".into()));
  }

  #[test]
  fn description_with_hashtags() {
    // Real Douyin title — `#` chain strips to "滴蒜苔人该下班"
    // (7 chars, no blacklist).
    assert_eq!(
      filter("滴蒜苔人该下班 #乡村田野 #蒜苔 #抽蒜苔 #田间地头走一走"),
      Some("滴蒜苔人该下班".into())
    );
  }

  #[test]
  fn long_title_with_colon() {
    // From dump: "国防部：敦促日方同军国主义彻底切割" — colon strip yields
    // 13 chars > 8, fails length on every converter.
    assert_eq!(filter("国防部：敦促日方同军国主义彻底切割"), None);
  }
}
