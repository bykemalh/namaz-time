use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderIds {
    pub imsakiyem: Option<String>,
    pub mtopal: Option<String>,
    pub ezanvakti: Option<String>,
}

impl ProviderIds {
    pub fn has_any(&self) -> bool {
        self.imsakiyem.is_some() || self.mtopal.is_some() || self.ezanvakti.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocationProfile {
    pub country_code: String,
    pub country_name: String,
    pub city_name: String,
    pub district_name: String,
    pub display_label: String,
    pub timezone: String,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub provider_ids: ProviderIds,
}

impl LocationProfile {
    pub fn is_turkey(&self) -> bool {
        self.country_code.eq_ignore_ascii_case("TR") || {
            let norm = normalize_name(&self.country_name);
            norm == "turkiye" || norm == "turkey"
        }
    }

    /// Stable, filesystem-safe cache key isolating every distinct location.
    pub fn cache_key(&self) -> String {
        let country = slugify_key(&self.country_name);
        let city = slugify_key(&self.city_name);
        let district = slugify_key(&self.district_name);
        let primary_id = self
            .provider_ids
            .imsakiyem
            .as_deref()
            .or(self.provider_ids.mtopal.as_deref())
            .or(self.provider_ids.ezanvakti.as_deref())
            .unwrap_or("none");
        let id_slug = slugify_key(primary_id);
        format!("{country}_{city}_{district}_{id_slug}")
    }

    pub fn short_title(&self) -> String {
        if self.district_name.is_empty() {
            self.city_name.clone()
        } else if normalize_name(&self.district_name) == normalize_name(&self.city_name) {
            if self.is_turkey() {
                self.district_name.clone()
            } else {
                format!("{}, {}", self.district_name, self.country_name)
            }
        } else {
            format!("{}, {}", self.district_name, self.city_name)
        }
    }
}

/// Lightweight entry returned by location search or district listing before full cross-provider matching.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocationCandidate {
    pub country_code: String,
    pub country_name: String,
    pub city_name: String,
    pub district_name: String,
    pub display_label: String,
    pub timezone: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub provider_ids: ProviderIds,
}

fn slugify_key(s: &str) -> String {
    let norm = normalize_name(s);
    let slug: String = norm
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if slug.is_empty() {
        "x".to_string()
    } else {
        slug
    }
}

/// Normalizes Turkish & European diacritics and folds `İ/I/ı/i` cleanly for cross-provider name matching.
pub fn normalize_name(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_space = true;

    for ch in input.trim().chars() {
        match ch {
            // Combining marks to drop
            '\u{0307}' | '\u{0300}'..='\u{036f}' => continue,
            'ı' | 'İ' | 'I' | 'i' | 'î' | 'Î' | 'í' | 'Í' | 'ì' | 'Ì' => {
                out.push('i');
                last_space = false;
            }
            'ş' | 'Ş' => {
                out.push('s');
                last_space = false;
            }
            'ğ' | 'Ğ' => {
                out.push('g');
                last_space = false;
            }
            'ü' | 'Ü' | 'û' | 'Û' | 'ú' | 'Ú' | 'ù' | 'Ù' => {
                out.push('u');
                last_space = false;
            }
            'ö' | 'Ö' | 'ô' | 'Ô' | 'ó' | 'Ó' | 'ò' | 'Ò' | 'ø' | 'Ø' => {
                out.push('o');
                last_space = false;
            }
            'ç' | 'Ç' => {
                out.push('c');
                last_space = false;
            }
            'â' | 'Â' | 'ä' | 'Ä' | 'á' | 'Á' | 'à' | 'À' | 'ã' | 'Ã' | 'å' | 'Å' => {
                out.push('a');
                last_space = false;
            }
            'é' | 'É' | 'è' | 'È' | 'ê' | 'Ê' | 'ë' | 'Ë' => {
                out.push('e');
                last_space = false;
            }
            'ñ' | 'Ñ' => {
                out.push('n');
                last_space = false;
            }
            'ß' => {
                out.push_str("ss");
                last_space = false;
            }
            c if c.is_ascii_alphanumeric() => {
                out.push(c.to_ascii_lowercase());
                last_space = false;
            }
            c if c.is_alphanumeric() => {
                for lc in c.to_lowercase() {
                    out.push(lc);
                }
                last_space = false;
            }
            _ => {
                if !last_space {
                    out.push(' ');
                    last_space = true;
                }
            }
        }
    }

    out.trim().to_string()
}

/// Generates normalized candidate aliases for country or city/district matching.
pub fn expand_name_aliases(name: &str, parent_city: Option<&str>) -> Vec<String> {
    let norm = normalize_name(name);
    let mut aliases = vec![norm.clone()];

    // Common country synonyms across providers
    match norm.as_str() {
        "turkiye" | "turkey" | "tr" => {
            aliases.push("turkiye".to_string());
            aliases.push("turkey".to_string());
        }
        "amerika birlesik devletleri" | "united states" | "usa" | "abd" => {
            aliases.push("abd".to_string());
            aliases.push("amerika birlesik devletleri".to_string());
            aliases.push("united states".to_string());
        }
        "kuzey kibris" | "kktc" | "north cyprus" => {
            aliases.push("kuzey kibris".to_string());
            aliases.push("kktc".to_string());
            aliases.push("north cyprus".to_string());
        }
        "birlesik krallik" | "ingiltere" | "united kingdom" | "uk" => {
            aliases.push("ingiltere".to_string());
            aliases.push("birlesik krallik".to_string());
            aliases.push("united kingdom".to_string());
        }
        _ => {}
    }

    if let Some(city) = parent_city {
        let city_norm = normalize_name(city);
        if norm == city_norm {
            aliases.push("merkez".to_string());
            aliases.push(format!("{city_norm} merkez"));
        } else if norm == format!("{city_norm} merkez") || norm == "merkez" {
            aliases.push(city_norm);
        }
    }

    aliases.sort();
    aliases.dedup();
    aliases
}

/// Matches a unique candidate by normalized name or alias. Returns `None` if ambiguous or missing.
pub fn match_unique_by_name<'a, T>(
    targets: &[String],
    items: &'a [T],
    extract_names: impl Fn(&'a T) -> Vec<String>,
) -> Option<&'a T> {
    if targets.is_empty() {
        return None;
    }

    // 1) Strict exact normalized match on primary targets
    let exact: Vec<&'a T> = items
        .iter()
        .filter(|item| {
            let names = extract_names(item);
            targets.iter().any(|t| names.iter().any(|n| n == t))
        })
        .collect();

    if exact.len() == 1 {
        return Some(exact[0]);
    }
    if exact.len() > 1 {
        // Prefer first exact match to primary target (targets[0]) if unique
        let primary = &targets[0];
        let primary_exact: Vec<&'a T> = exact
            .iter()
            .copied()
            .filter(|item| extract_names(item).iter().any(|n| n == primary))
            .collect();
        if primary_exact.len() == 1 {
            return Some(primary_exact[0]);
        }
        return None;
    }

    None
}

/// Fallback timezone mapping for major countries when Imsakiyem district timezone is unavailable.
pub fn fallback_timezone_for_country(country_code: &str, country_name: &str) -> Option<&'static str> {
    let code = country_code.trim().to_ascii_uppercase();
    match code.as_str() {
        "TR" => return Some("Europe/Istanbul"),
        "CY" => return Some("Asia/Nicosia"),
        "DE" => return Some("Europe/Berlin"),
        "AT" => return Some("Europe/Vienna"),
        "CH" => return Some("Europe/Zurich"),
        "FR" => return Some("Europe/Paris"),
        "NL" => return Some("Europe/Amsterdam"),
        "BE" => return Some("Europe/Brussels"),
        "GB" => return Some("Europe/London"),
        "IT" => return Some("Europe/Rome"),
        "ES" => return Some("Europe/Madrid"),
        "SE" => return Some("Europe/Stockholm"),
        "NO" => return Some("Europe/Oslo"),
        "DK" => return Some("Europe/Copenhagen"),
        "AZ" => return Some("Asia/Baku"),
        "SA" => return Some("Asia/Riyadh"),
        "AE" => return Some("Asia/Dubai"),
        "QA" => return Some("Asia/Qatar"),
        "JP" => return Some("Asia/Tokyo"),
        "BA" => return Some("Europe/Sarajevo"),
        "BG" => return Some("Europe/Sofia"),
        "GR" => return Some("Europe/Athens"),
        "MC" => return Some("Europe/Monaco"),
        _ => {}
    }

    match normalize_name(country_name).as_str() {
        "turkiye" | "turkey" => Some("Europe/Istanbul"),
        "kuzey kibris" | "kktc" | "north cyprus" => Some("Asia/Nicosia"),
        "almanya" | "germany" => Some("Europe/Berlin"),
        "hollanda" | "netherlands" => Some("Europe/Amsterdam"),
        "fransa" | "france" => Some("Europe/Paris"),
        "belcika" | "belgium" => Some("Europe/Brussels"),
        "avusturya" | "austria" => Some("Europe/Vienna"),
        "isvicre" | "switzerland" => Some("Europe/Zurich"),
        "ingiltere" | "birlesik krallik" | "united kingdom" => Some("Europe/London"),
        "azerbaycan" | "azerbaijan" => Some("Asia/Baku"),
        _ => None,
    }
}

pub fn system_timezone_iana() -> String {
    jiff::tz::TimeZone::system()
        .iana_name()
        .unwrap_or("UTC")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_turkish_and_german_names() {
        assert_eq!(normalize_name("İstanbul"), "istanbul");
        assert_eq!(normalize_name("İSTANBUL"), "istanbul");
        assert_eq!(normalize_name("Şanlıurfa"), "sanliurfa");
        assert_eq!(normalize_name("Ağrı"), "agri");
        assert_eq!(normalize_name("Überlingen"), "uberlingen");
        assert_eq!(normalize_name("München"), "munchen");
        assert_eq!(normalize_name("Baden-Württemberg"), "baden wurttemberg");
    }

    #[test]
    fn matches_istanbul_merkez_aliases() {
        let items = vec!["Arnavutköy", "Kadıköy", "İstanbul"];
        let targets = expand_name_aliases("İstanbul Merkez", Some("İstanbul"));
        let found = match_unique_by_name(&targets, &items, |s| expand_name_aliases(s, Some("İstanbul")));
        assert_eq!(found, Some(&"İstanbul"));
    }

    #[test]
    fn rejects_ambiguous_match() {
        let items = vec!["Merkez", "Merkez"];
        let targets = vec!["merkez".to_string()];
        let found = match_unique_by_name(&targets, &items, |s| vec![normalize_name(s)]);
        assert!(found.is_none());
    }
}
