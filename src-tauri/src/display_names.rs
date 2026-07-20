pub fn map_name_for_display(value: &str) -> Option<String> {
    let candidate = internal_asset_name(value).unwrap_or_else(|| value.trim().to_string());
    normalize_lookup_value(&candidate).map(|candidate| {
        known_map_name_for_display(candidate).unwrap_or_else(|| candidate.to_string())
    })
}

pub fn known_map_name_for_display(value: &str) -> Option<String> {
    let candidate = internal_asset_name(value).unwrap_or_else(|| value.trim().to_string());
    let candidate = normalize_lookup_value(&candidate)?;
    lookup_display_name(candidate, MAP_NAME_ALIASES).map(str::to_string)
}

pub fn is_obsolete_map_display_name(value: &str) -> bool {
    matches!(value.trim(), "幽邃迷境" | "迷邃幽境")
}

pub fn game_mode_for_display(value: &str) -> Option<String> {
    let candidate = internal_asset_name(value).unwrap_or_else(|| value.trim().to_string());
    normalize_lookup_value(&candidate).and_then(|candidate| {
        if candidate
            .chars()
            .all(|character| character.is_ascii_digit())
        {
            return None;
        }

        Some(
            lookup_display_name(candidate, GAME_MODE_ALIASES)
                .unwrap_or(candidate)
                .to_string(),
        )
    })
}

pub fn player_name_for_display(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || looks_like_asset_reference(trimmed) {
        return None;
    }

    if let Some((name, tag)) = trimmed.split_once('#') {
        let name = name.trim();
        let tag = tag.trim().trim_start_matches('#').trim();
        if name.is_empty() || tag.is_empty() {
            return None;
        }
        return Some(format!("{name}#{tag}"));
    }

    Some(trimmed.to_string())
}

pub fn agent_name_for_display(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if let Some(agent_name) = known_agent_name_for_display(trimmed) {
        return Some(agent_name);
    }

    let candidate = internal_asset_name(trimmed)?;
    known_agent_name_for_display(&candidate)
}

fn known_agent_name_for_display(value: &str) -> Option<String> {
    let candidate = normalize_lookup_value(value)?;

    lookup_display_name(candidate, AGENT_NAME_ALIASES)
        .map(str::to_string)
        .or_else(|| {
            LOCALIZED_AGENT_NAMES
                .iter()
                .find(|agent_name| agent_name.eq_ignore_ascii_case(candidate))
                .map(|agent_name| (*agent_name).to_string())
        })
}

/// Returns the localized agent label used by the frontend and library facets.
///
/// Stored metadata intentionally keeps the source-facing agent value (for example `Jett`),
/// while the UI presents the mainland Chinese name (`捷风`). Keeping this conversion here lets
/// facet aggregation and filtering share the same canonical label without rewriting old rows.
pub fn localized_agent_name_for_display(value: &str) -> Option<String> {
    let canonical_name = agent_name_for_display(value)?;
    lookup_display_name(&canonical_name, LOCALIZED_AGENT_NAME_ALIASES)
        .map(str::to_string)
        .or_else(|| {
            LOCALIZED_AGENT_NAMES
                .iter()
                .find(|agent_name| agent_name.eq_ignore_ascii_case(&canonical_name))
                .map(|agent_name| (*agent_name).to_string())
        })
}

/// Expands a localized or source-facing agent selection to every stored alias that represents it.
pub fn agent_name_filter_values(value: &str) -> Vec<String> {
    let Some(localized_name) = localized_agent_name_for_display(value) else {
        return Vec::new();
    };
    let mut values = vec![localized_name.clone()];

    for (alias, _canonical_name) in AGENT_NAME_ALIASES {
        if localized_agent_name_for_display(alias).as_deref() == Some(localized_name.as_str())
            && !values.iter().any(|value| value.eq_ignore_ascii_case(alias))
        {
            values.push((*alias).to_string());
        }
    }

    values
}

pub fn agent_name_from_export_text(value: &str) -> Option<String> {
    let lower = value.replace('\\', "/").to_ascii_lowercase();
    ACLOS_AGENT_ASSET_IDS
        .iter()
        .find(|(asset_id, _agent_name)| {
            lower.contains(&format!("agentbackground/agent/{asset_id}.png"))
                || lower.contains(&format!("agentskill/{asset_id}_"))
        })
        .map(|(_asset_id, agent_name)| (*agent_name).to_string())
}

pub fn agent_name_from_asset_id(value: &str) -> Option<String> {
    let normalized = value.trim().trim_start_matches('0');
    let normalized = if normalized.is_empty() {
        "0"
    } else {
        normalized
    };

    ACLOS_AGENT_ASSET_IDS
        .iter()
        .find(|(asset_id, _agent_name)| asset_id.trim_start_matches('0') == normalized)
        .map(|(_asset_id, agent_name)| (*agent_name).to_string())
}

pub fn agent_name_from_avatar_url(value: &str) -> Option<String> {
    let lower = value.trim().replace('\\', "/").to_ascii_lowercase();
    ["headico/", "headicon/", "bigpic/"]
        .into_iter()
        .find_map(|marker| {
            let start = lower.find(marker)? + marker.len();
            let asset_id = lower[start..]
                .chars()
                .take_while(|character| character.is_ascii_digit())
                .collect::<String>();
            agent_name_from_asset_id(&asset_id)
        })
        .or_else(|| agent_name_from_export_text(value))
}

pub fn weapon_name_for_display(value: &str) -> Option<String> {
    let candidate = weapon_asset_name(value)?;
    WEAPON_NAME_ALIASES
        .iter()
        .find(|(alias, _display)| {
            candidate.eq_ignore_ascii_case(alias)
                || candidate
                    .get(alias.len()..)
                    .is_some_and(|suffix| suffix.starts_with('_'))
                    && candidate[..alias.len()].eq_ignore_ascii_case(alias)
        })
        .map(|(_alias, display)| (*display).to_string())
        .or(Some(candidate))
}

fn weapon_asset_name(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        return None;
    }

    let normalized = trimmed.replace('\\', "/");
    let tail = normalized
        .rsplit('/')
        .next()
        .unwrap_or(trimmed)
        .split('.')
        .next_back()
        .unwrap_or(trimmed)
        .trim_start_matches("Default__")
        .split("_C_")
        .next()
        .unwrap_or(trimmed)
        .trim_end_matches("_C")
        .trim();

    if tail.is_empty() {
        None
    } else {
        Some(tail.to_string())
    }
}

pub fn looks_like_asset_reference(value: &str) -> bool {
    let lower = value.trim().replace('\\', "/").to_ascii_lowercase();

    lower.starts_with("/game/")
        || lower.starts_with("cards/")
        || lower.contains("/cards/")
        || lower.contains("card: cards/")
        || lower.contains(":persistentlevel.")
        || lower.contains("_primaryasset")
        || lower.contains("_primarydataasset")
        || lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".webp")
}

pub fn internal_asset_name(value: &str) -> Option<String> {
    let original = value.trim();
    let candidate = path_tail_name(original)?
        .split('.')
        .next_back()
        .unwrap_or(original)
        .trim_start_matches("Default__")
        .trim_end_matches("_C")
        .trim_end_matches("_PrimaryAsset")
        .trim_end_matches("_PrimaryDataAsset")
        .split('_')
        .next()
        .unwrap_or(original)
        .trim();

    if candidate.is_empty() || candidate == original {
        None
    } else {
        Some(candidate.to_string())
    }
}

pub fn path_tail_name(value: &str) -> Option<&str> {
    value
        .trim_matches('/')
        .rsplit('/')
        .next()
        .and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
}

fn normalize_lookup_value(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() || looks_like_asset_reference(trimmed) {
        None
    } else {
        Some(trimmed)
    }
}

fn lookup_display_name<'a>(value: &str, aliases: &'a [(&str, &str)]) -> Option<&'a str> {
    aliases
        .iter()
        .find(|(alias, _display)| alias.eq_ignore_ascii_case(value))
        .map(|(_alias, display)| *display)
}

const MAP_NAME_ALIASES: &[(&str, &str)] = &[
    ("Plummet", "天枢云阙"),
    ("Summit", "天枢云阙"),
    ("Rook", "盐海矿镇"),
    ("Corrode", "盐海矿镇"),
    ("Infinity", "幽邃地窟"),
    ("Abyss", "幽邃地窟"),
    ("Juliett", "日落之城"),
    ("Sunset", "日落之城"),
    ("Jam", "莲华古城"),
    ("Lotus", "莲华古城"),
    ("Pitt", "深海明珠"),
    ("Pearl", "深海明珠"),
    ("Canyon", "裂变峡谷"),
    ("Fracture", "裂变峡谷"),
    ("Foxtrot", "微风岛屿"),
    ("Breeze", "微风岛屿"),
    ("Port", "森寒冬港"),
    ("Icebox", "森寒冬港"),
    ("Ascent", "亚海悬城"),
    ("Bonsai", "霓虹町"),
    ("Split", "霓虹町"),
    ("Triad", "隐世修所"),
    ("Haven", "隐世修所"),
    ("Duality", "源工重镇"),
    ("Bind", "源工重镇"),
];

const GAME_MODE_ALIASES: &[(&str, &str)] = &[
    ("BombGameMode", "竞技模式"),
    ("competitive", "竞技模式"),
    ("Competitive", "竞技模式"),
    ("Unrated", "未评级"),
    ("unrated", "未评级"),
    ("Swiftplay", "极速模式"),
    ("Deathmatch", "死斗模式"),
    ("DeathmatchGameMode", "死斗模式"),
    ("SkirmishGameMode", "团队乱斗"),
    ("Team Deathmatch", "团队乱斗"),
    ("TeamDeathmatch", "团队乱斗"),
    ("Spike Rush", "乱斗模式"),
    ("SpikeRush", "乱斗模式"),
    ("Custom Game", "自定义游戏"),
    ("CustomGame", "自定义游戏"),
];

const AGENT_NAME_ALIASES: &[(&str, &str)] = &[
    ("Astra", "Astra"),
    ("Breach", "Breach"),
    ("Brimstone", "Brimstone"),
    ("Chamber", "Chamber"),
    ("Clove", "Clove"),
    ("Cypher", "Cypher"),
    ("Deadlock", "Deadlock"),
    ("Fade", "Fade"),
    ("Gekko", "Gekko"),
    ("Harbor", "Harbor"),
    ("Iso", "Iso"),
    ("Jett", "Jett"),
    ("KAY/O", "KAY/O"),
    ("Kayo", "KAY/O"),
    ("Killjoy", "Killjoy"),
    ("Miks", "Miks"),
    ("Neon", "Neon"),
    ("Omen", "Omen"),
    ("Phoenix", "Phoenix"),
    ("Raze", "Raze"),
    ("Reyna", "Reyna"),
    ("Sage", "Sage"),
    ("Skye", "Skye"),
    ("Sova", "Sova"),
    ("Tejo", "Tejo"),
    ("Veto", "Veto"),
    ("Viper", "Viper"),
    ("Vyse", "Vyse"),
    ("Waylay", "Waylay"),
    ("Yoru", "Yoru"),
    ("Vampire", "Reyna"),
    ("Nox", "Vyse"),
    ("Deadeye", "Chamber"),
];

const LOCALIZED_AGENT_NAME_ALIASES: &[(&str, &str)] = &[
    ("Brimstone", "炼狱"),
    ("Viper", "蝰蛇"),
    ("Omen", "幽影"),
    ("Killjoy", "奇乐"),
    ("Cypher", "零"),
    ("Sova", "猎枭"),
    ("Sage", "贤者"),
    ("Phoenix", "不死鸟"),
    ("Jett", "捷风"),
    ("Reyna", "芮娜"),
    ("Raze", "雷兹"),
    ("Breach", "铁臂"),
    ("Skye", "斯凯"),
    ("Yoru", "夜露"),
    ("Astra", "星礈"),
    ("KAY/O", "K/O"),
    ("Chamber", "尚勃勒"),
    ("Neon", "霓虹"),
    ("Fade", "黑梦"),
    ("Harbor", "海神"),
    ("Gekko", "盖可"),
    ("Deadlock", "钢锁"),
    ("Iso", "壹决"),
    ("Clove", "暮蝶"),
    ("Vyse", "维斯"),
    ("Tejo", "钛狐"),
    ("Miks", "迷核"),
    ("Waylay", "幻棱"),
    ("Veto", "禁灭"),
];

const WEAPON_NAME_ALIASES: &[(&str, &str)] = &[
    ("AssaultRifle_Burst", "獠犬"),
    ("AssaultRifle_ACR", "幻影"),
    ("AssaultRifle_AK", "狂徒"),
    ("SubMachineGun_MP5", "骇灵"),
    ("AutomaticShotgun", "判官"),
    ("RevolverPistol", "正义"),
    ("LeverSniperRifle", "飞将"),
    ("SawedOffShotgun", "短炮"),
    ("HeavyMachineGun", "奥丁"),
    ("LightMachineGun", "战神"),
    ("CompactPistol", "狂怒"),
    ("PumpShotgun", "雄鹿"),
    ("LugerPistol", "鬼魅"),
    ("BoltSniper", "冥驹"),
    ("BasePistol", "标配"),
    ("Vector", "蜂刺"),
    ("DMR", "戍卫"),
    ("Bulldog", "獠犬"),
    ("Phantom", "幻影"),
    ("Vandal", "狂徒"),
    ("Spectre", "骇灵"),
    ("Judge", "判官"),
    ("Sheriff", "正义"),
    ("Marshal", "飞将"),
    ("Shorty", "短炮"),
    ("Odin", "奥丁"),
    ("Ares", "战神"),
    ("Frenzy", "狂怒"),
    ("Bucky", "雄鹿"),
    ("Ghost", "鬼魅"),
    ("Operator", "冥驹"),
    ("Classic", "标配"),
    ("Stinger", "蜂刺"),
    ("Guardian", "戍卫"),
];

const LOCALIZED_AGENT_NAMES: &[&str] = &[
    "炼狱",
    "蝰蛇",
    "幽影",
    "奇乐",
    "零",
    "猎枭",
    "贤者",
    "不死鸟",
    "捷风",
    "芮娜",
    "雷兹",
    "铁臂",
    "斯凯",
    "夜露",
    "星礈",
    "K/O",
    "尚勃勒",
    "霓虹",
    "黑梦",
    "海神",
    "盖可",
    "钢锁",
    "壹决",
    "暮蝶",
    "维斯",
    "钛狐",
    "迷核",
    "幻棱",
    "禁灭",
];

const ACLOS_AGENT_ASSET_IDS: &[(&str, &str)] = &[
    ("02", "Jett"),
    ("03", "Raze"),
    ("04", "Omen"),
    ("06", "Phoenix"),
    ("07", "Sage"),
    ("08", "Sova"),
    ("10", "Cypher"),
    ("11", "Reyna"),
    ("13", "Skye"),
    ("16", "KAY/O"),
    ("17", "Chamber"),
    ("18", "Neon"),
    ("23", "Iso"),
    ("24", "Clove"),
    ("25", "Vyse"),
    ("29", "Veto"),
];

#[cfg(test)]
mod tests {
    use super::{
        agent_name_filter_values, localized_agent_name_for_display, map_name_for_display,
        weapon_name_for_display,
    };

    #[test]
    fn localizes_agent_aliases_and_expands_them_for_library_filters() {
        assert_eq!(
            localized_agent_name_for_display("Jett").as_deref(),
            Some("捷风")
        );
        assert_eq!(
            localized_agent_name_for_display(
                "/Game/Characters/Jett/Jett_PrimaryAsset.Jett_PrimaryAsset_C"
            )
            .as_deref(),
            Some("捷风")
        );
        assert_eq!(
            localized_agent_name_for_display("Vampire").as_deref(),
            Some("芮娜")
        );
        assert_eq!(
            localized_agent_name_for_display("KAY/O").as_deref(),
            Some("K/O")
        );
        assert_eq!(
            localized_agent_name_for_display("K/O").as_deref(),
            Some("K/O")
        );
        assert_eq!(
            localized_agent_name_for_display("Miks").as_deref(),
            Some("迷核")
        );
        assert!(localized_agent_name_for_display("unknown-agent").is_none());

        let jett_values = agent_name_filter_values("捷风");
        assert!(jett_values.iter().any(|value| value == "捷风"));
        assert!(jett_values.iter().any(|value| value == "Jett"));

        let reyna_values = agent_name_filter_values("Reyna");
        assert!(reyna_values.iter().any(|value| value == "芮娜"));
        assert!(reyna_values.iter().any(|value| value == "Reyna"));
        assert!(reyna_values.iter().any(|value| value == "Vampire"));

        let kayo_values = agent_name_filter_values("K/O");
        assert!(kayo_values.iter().any(|value| value == "K/O"));
        assert!(kayo_values.iter().any(|value| value == "KAY/O"));
        assert!(kayo_values.iter().any(|value| value == "Kayo"));
    }

    #[test]
    fn maps_every_standard_map_alias_to_the_mainland_chinese_name() {
        for (alias, expected) in [
            ("/Game/Maps/Plummet/Plummet", "天枢云阙"),
            ("Summit", "天枢云阙"),
            ("Rook", "盐海矿镇"),
            ("Corrode", "盐海矿镇"),
            ("Infinity", "幽邃地窟"),
            ("Abyss", "幽邃地窟"),
            ("Juliett", "日落之城"),
            ("Sunset", "日落之城"),
            ("Jam", "莲华古城"),
            ("Lotus", "莲华古城"),
            ("Pitt", "深海明珠"),
            ("Pearl", "深海明珠"),
            ("Canyon", "裂变峡谷"),
            ("Fracture", "裂变峡谷"),
            ("Foxtrot", "微风岛屿"),
            ("Breeze", "微风岛屿"),
            ("Port", "森寒冬港"),
            ("Icebox", "森寒冬港"),
            ("Ascent", "亚海悬城"),
            ("Bonsai", "霓虹町"),
            ("Split", "霓虹町"),
            ("Triad", "隐世修所"),
            ("Haven", "隐世修所"),
            ("Duality", "源工重镇"),
            ("Bind", "源工重镇"),
        ] {
            assert_eq!(
                map_name_for_display(alias).as_deref(),
                Some(expected),
                "alias {alias}"
            );
        }
    }

    #[test]
    fn maps_wonderful_weapon_paths_and_skin_assets_to_display_names() {
        for (value, expected) in [
            (
                "/Game/Maps/Pitt/Pitt.Pitt:PersistentLevel.AssaultRifle_AK_C_2147000127",
                "狂徒",
            ),
            (
                "AssaultRifle_ACR_SpecOps_PrimaryAsset_C /Game/Equippables/Guns/Rifles/AssaultRifle_ACR/SpecOps/AssaultRifle_ACR_SpecOps_PrimaryAsset.Default__AssaultRifle_ACR_SpecOps_PrimaryAsset_C",
                "幻影",
            ),
            (
                "/Game/Maps/Duality/Duality.Duality:PersistentLevel.LugerPistol_C_2147311741",
                "鬼魅",
            ),
        ] {
            assert_eq!(
                weapon_name_for_display(value).as_deref(),
                Some(expected),
                "weapon value {value}"
            );
        }
    }
}
