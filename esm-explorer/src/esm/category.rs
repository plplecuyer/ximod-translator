//! Map weapon-type keyword EditorIDs to a human-readable function category.
//!
//! Bethesda games tag weapons with keyword records whose EditorID names the
//! type: Skyrim uses `WeapTypeSword`, `WeapTypeBow`… ; Fallout 4 / Starfield use
//! `WeaponTypePistol`, `WeaponTypeRifle`, `WeaponTypeMelee`… The matching is a
//! case-insensitive substring test in priority order, so the most specific label
//! wins. Returns `None` when no keyword identifies a weapon type.

/// French category for a set of a weapon's keyword EditorIDs.
pub fn weapon_category(edids: &[String]) -> Option<&'static str> {
    // Lowercased once for substring tests.
    let low: Vec<String> = edids.iter().map(|e| e.to_lowercase()).collect();
    let any = |needle: &str| low.iter().any(|e| e.contains(needle));

    // Order matters: check the more specific/telling tokens first.
    // --- firearms (Fallout / Starfield) ---
    if any("shotgun") {
        return Some("Arme à feu — Fusil à pompe");
    }
    if any("sniper") {
        return Some("Arme à feu — Fusil de précision");
    }
    if any("pistol") || any("sidearm") {
        return Some("Arme à feu — Arme de poing");
    }
    if any("rifle") {
        return Some("Arme à feu — Fusil");
    }
    if any("heavy") || any("minigun") || any("launcher") {
        return Some("Arme à feu — Arme lourde");
    }
    if any("grenade") || any("mine") || any("explosive") {
        return Some("Explosif");
    }
    // --- Skyrim melee & ranged ---
    if any("greatsword") {
        return Some("Arme blanche — Épée à deux mains");
    }
    if any("battleaxe") {
        return Some("Arme blanche — Hache de guerre");
    }
    if any("warhammer") {
        return Some("Arme blanche — Marteau de guerre");
    }
    if any("waraxe") {
        return Some("Arme blanche — Hache");
    }
    if any("dagger") {
        return Some("Arme blanche — Dague");
    }
    if any("sword") || any("scimitar") {
        return Some("Arme blanche — Épée");
    }
    if any("mace") {
        return Some("Arme blanche — Masse");
    }
    if any("crossbow") {
        return Some("Arme à distance — Arbalète");
    }
    if any("bow") {
        return Some("Arme à distance — Arc");
    }
    if any("staff") {
        return Some("Bâton");
    }
    // --- generic melee fallbacks ---
    if any("melee1h") || any("melee2h") || any("melee") {
        return Some("Arme de mêlée");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn firearms_and_melee() {
        assert_eq!(
            weapon_category(&["WeaponTypePistol".into()]),
            Some("Arme à feu — Arme de poing")
        );
        assert_eq!(
            weapon_category(&["WeaponTypeRifle".into()]),
            Some("Arme à feu — Fusil")
        );
        assert_eq!(
            weapon_category(&["WeapTypeGreatsword".into(), "VendorItemWeapon".into()]),
            Some("Arme blanche — Épée à deux mains")
        );
        assert_eq!(
            weapon_category(&["WeapTypeDagger".into()]),
            Some("Arme blanche — Dague")
        );
        assert_eq!(
            weapon_category(&["WeapTypeBow".into()]),
            Some("Arme à distance — Arc")
        );
        assert_eq!(weapon_category(&["SomeUnrelatedKeyword".into()]), None);
    }
}
