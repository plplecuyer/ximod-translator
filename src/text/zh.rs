//! Simplified ↔ traditional Chinese (E-08, D-100): the conversions of OpenCC
//! (characters and expressions, Taiwan and Hong Kong variants) through
//! ferrous-opencc, which carries its tables. Tags, aliases, parameters and
//! numbers are not converted (the segments of A-13).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use ferrous_opencc::OpenCC;
use ferrous_opencc::config::BuiltinConfig;

use crate::text::transform::{self, Piece};

/// A conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ZhConv {
    /// Simplified → traditional (standard OpenCC).
    S2t,
    /// Simplified → traditional of Taiwan, with its expressions.
    S2tw,
    /// Simplified → traditional of Hong Kong.
    S2hk,
    /// Traditional → simplified.
    T2s,
    /// Traditional of Taiwan, with its expressions → simplified.
    Tw2s,
    /// Traditional of Hong Kong → simplified.
    Hk2s,
}

impl ZhConv {
    pub const ALL: [ZhConv; 6] = [
        ZhConv::S2t,
        ZhConv::S2tw,
        ZhConv::S2hk,
        ZhConv::T2s,
        ZhConv::Tw2s,
        ZhConv::Hk2s,
    ];

    /// The conversions to traditional, those an export offers.
    pub const TO_TRADITIONAL: [ZhConv; 3] = [ZhConv::S2t, ZhConv::S2tw, ZhConv::S2hk];

    /// The name on the command line (`--convert s2tw`).
    pub fn key(self) -> &'static str {
        match self {
            ZhConv::S2t => "s2t",
            ZhConv::S2tw => "s2tw",
            ZhConv::S2hk => "s2hk",
            ZhConv::T2s => "t2s",
            ZhConv::Tw2s => "tw2s",
            ZhConv::Hk2s => "hk2s",
        }
    }

    pub fn from_key(k: &str) -> Option<ZhConv> {
        ZhConv::ALL
            .into_iter()
            .find(|c| c.key().eq_ignore_ascii_case(k.trim()))
    }

    /// The message key of its name.
    pub fn i18n_key(self) -> &'static str {
        match self {
            ZhConv::S2t => "zh-s2t",
            ZhConv::S2tw => "zh-s2tw",
            ZhConv::S2hk => "zh-s2hk",
            ZhConv::T2s => "zh-t2s",
            ZhConv::Tw2s => "zh-tw2s",
            ZhConv::Hk2s => "zh-hk2s",
        }
    }

    fn builtin(self) -> BuiltinConfig {
        match self {
            ZhConv::S2t => BuiltinConfig::S2t,
            ZhConv::S2tw => BuiltinConfig::S2twp,
            ZhConv::S2hk => BuiltinConfig::S2hk,
            ZhConv::T2s => BuiltinConfig::T2s,
            ZhConv::Tw2s => BuiltinConfig::Tw2sp,
            ZhConv::Hk2s => BuiltinConfig::Hk2s,
        }
    }
}

/// The converter of a conversion, built once.
fn converter(conv: ZhConv) -> Option<&'static OpenCC> {
    static CACHE: OnceLock<Mutex<HashMap<ZhConv, &'static OpenCC>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let mut map = cache.lock().ok()?;
    if let Some(c) = map.get(&conv) {
        return Some(c);
    }
    match OpenCC::from_config(conv.builtin()) {
        Ok(c) => {
            let c: &'static OpenCC = Box::leak(Box::new(c));
            map.insert(conv, c);
            Some(c)
        }
        Err(e) => {
            tracing::error!("OpenCC {}: {e}", conv.key());
            None
        }
    }
}

/// Whether a language (ISO 639-3) is Chinese: the command and the export
/// option apply.
pub fn is_chinese(iso3: &str) -> bool {
    matches!(
        iso3,
        "zho" | "cmn" | "yue" | "wuu" | "hak" | "nan" | "gan" | "hsn" | "cjy" | "cdo" | "cpx"
    )
}

/// Convert a text, its tags, numbers and parameters kept as they are.
pub fn convert(conv: ZhConv, text: &str) -> String {
    let Some(cc) = converter(conv) else {
        return text.to_string();
    };
    transform::pieces(text)
        .into_iter()
        .map(|p| match p {
            Piece::Text(t) => cc.convert(t),
            Piece::Kept(k) => k.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-Z: « 软件 » is « 軟體 » in Taiwan, « 軟件 » in standard.
    #[test]
    fn acceptance_t_z_software() {
        assert_eq!(convert(ZhConv::S2tw, "软件"), "軟體");
        assert_eq!(convert(ZhConv::S2t, "软件"), "軟件");
        assert_eq!(convert(ZhConv::S2hk, "软件"), "軟件");
        assert_eq!(convert(ZhConv::T2s, "軟件"), "软件");
        assert_eq!(convert(ZhConv::Tw2s, "軟體"), "软件");
    }

    /// The tags, aliases, parameters and numbers are kept.
    #[test]
    fn tags_and_numbers_are_kept() {
        let s = "对<Alias=Target>造成<mag>点伤害，持续{0}秒，共25次。%s";
        let t = convert(ZhConv::S2t, s);
        for kept in ["<Alias=Target>", "<mag>", "{0}", "25", "%s"] {
            assert!(t.contains(kept), "{kept} in {t}");
        }
        assert!(t.contains("對") && t.contains("點傷害"), "{t}");
    }

    /// T-Z: a round trip loses nothing on sentences of a game.
    #[test]
    fn acceptance_t_z_round_trip() {
        let sentences = [
            "你的魔法值不足。",
            "召唤一个火焰元素，持续<dur>秒。",
            "对目标造成<mag>点冰霜伤害。",
            "解锁专家法术（250000金币）",
            "生命值 +25（5000金币）",
            "你感到更加强大了。",
            "这扇门被锁住了。",
            "选择一个技能树。",
            "保存游戏",
            "读取存档",
            "设置",
            "退出到主菜单",
            "确定要退出吗？",
            "装备",
            "物品",
            "魔法",
            "地图",
            "任务已完成",
            "新任务：寻找古老的卷轴",
            "你获得了<Alias=Item>。",
            "天际省",
            "龙裔",
            "雪漫城",
            "冬堡学院",
            "盗贼公会",
            "黑暗兄弟会",
            "战士公会",
            "法师公会",
            "神秘的陌生人",
            "古老的龙语",
            "你学会了一个新的龙吼。",
            "治疗术",
            "火球术",
            "闪电链",
            "冰霜之矛",
            "隐身术",
            "召唤骷髅",
            "恢复生命值<mag>点。",
            "移动速度提高<mag>%。",
            "负重增加<4>倍。",
            "持续时间：<dur>秒",
            "附魔",
            "炼金",
            "锻造",
            "开锁",
            "潜行",
            "口才",
            "单手武器",
            "双手武器",
            "重甲",
        ];
        assert_eq!(sentences.len(), 50);
        for s in sentences {
            let t = convert(ZhConv::S2t, s);
            assert_eq!(convert(ZhConv::T2s, &t), s, "{s} → {t}");
        }
    }

    #[test]
    fn keys_and_languages() {
        for c in ZhConv::ALL {
            assert_eq!(ZhConv::from_key(c.key()), Some(c));
        }
        assert_eq!(ZhConv::from_key("S2TW"), Some(ZhConv::S2tw));
        assert_eq!(ZhConv::from_key("x"), None);
        assert!(is_chinese("zho") && !is_chinese("jpn"));
    }
}
