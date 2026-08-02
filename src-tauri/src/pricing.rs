use crate::{
    models::{BillingMode, PricingMatchKind, PricingScopeKind},
    usage_log::TokenUsage,
};

pub(crate) const OFFICIAL_PRICING_RULE_VERSION: i64 = 20260801;
const OFFICIAL_PRICING_RULE_PREFIX: &str = "openai-official-standard";
const OFFICIAL_PRICING_RULE_NAME: &str = "OpenAI 官方参考价";

#[derive(Debug, Clone)]
pub(crate) struct PricingRuleRecord {
    pub id: String,
    pub version: i64,
    pub active: bool,
    pub scope_kind: PricingScopeKind,
    pub provider_id: Option<String>,
    pub account_id: Option<String>,
    pub model_pattern: String,
    pub match_kind: PricingMatchKind,
    pub billing_mode: BillingMode,
    pub input_microusd_per_million: Option<i64>,
    pub cached_read_microusd_per_million: Option<i64>,
    pub cache_write_microusd_per_million: Option<i64>,
    pub output_microusd_per_million: Option<i64>,
    pub request_fee_microusd: Option<i64>,
    pub cache_write_included_in_input: bool,
    pub effective_from_ms: i64,
}

impl PricingRuleRecord {
    #[cfg(test)]
    fn token(
        id: &str,
        scope_kind: PricingScopeKind,
        scope_id: Option<&str>,
        input: i64,
        cached_read: i64,
        cache_write: i64,
        output: i64,
    ) -> Self {
        Self {
            id: id.into(),
            version: 1,
            active: true,
            scope_kind,
            provider_id: (scope_kind != PricingScopeKind::GlobalModel)
                .then(|| scope_id.unwrap_or_default().into()),
            account_id: (scope_kind == PricingScopeKind::AccountModel)
                .then(|| scope_id.unwrap_or_default().into()),
            model_pattern: id.into(),
            match_kind: PricingMatchKind::Exact,
            billing_mode: BillingMode::Token,
            input_microusd_per_million: Some(input),
            cached_read_microusd_per_million: Some(cached_read),
            cache_write_microusd_per_million: Some(cache_write),
            output_microusd_per_million: Some(output),
            request_fee_microusd: None,
            cache_write_included_in_input: true,
            effective_from_ms: 0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PricingContext<'a> {
    pub model: &'a str,
    pub provider_id: Option<&'a str>,
    pub account_id: Option<&'a str>,
    pub effective_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PricingOutcome {
    Estimated {
        cost_microusd: i64,
        rule_id: String,
        version: i64,
    },
    Subscription {
        rule_id: String,
        version: i64,
    },
    Unpriced {
        rule_id: Option<String>,
        reason: String,
    },
}

pub(crate) fn price_for_source(
    source_kind: crate::models::UsageSourceKind,
    rules: &[PricingRuleRecord],
    usage: &TokenUsage,
    context: &PricingContext<'_>,
) -> PricingOutcome {
    if source_kind == crate::models::UsageSourceKind::Official {
        return price_with_official_catalog(usage, context);
    }
    price_with_rules(rules, usage, context)
}

pub(crate) fn price_with_official_catalog(
    usage: &TokenUsage,
    context: &PricingContext<'_>,
) -> PricingOutcome {
    let normalized = normalize_model_name(context.model);
    let Some((model_name, input, cached_read, cache_write, output)) = official_rates(&normalized)
    else {
        return PricingOutcome::Unpriced {
            rule_id: None,
            reason: "官方价格目录暂未收录此模型。".into(),
        };
    };
    let rule_id = format!("{OFFICIAL_PRICING_RULE_PREFIX}:{model_name}");
    let rule = PricingRuleRecord {
        id: rule_id,
        version: OFFICIAL_PRICING_RULE_VERSION,
        active: true,
        scope_kind: PricingScopeKind::GlobalModel,
        provider_id: None,
        account_id: None,
        model_pattern: model_name.to_owned(),
        match_kind: PricingMatchKind::Exact,
        billing_mode: BillingMode::Token,
        input_microusd_per_million: Some(input),
        cached_read_microusd_per_million: Some(cached_read),
        cache_write_microusd_per_million: Some(cache_write),
        output_microusd_per_million: Some(output),
        request_fee_microusd: None,
        cache_write_included_in_input: true,
        effective_from_ms: 0,
    };
    calculate(&rule, usage, context)
}

pub(crate) fn official_pricing_rule_name(id: &str) -> Option<String> {
    id.starts_with(OFFICIAL_PRICING_RULE_PREFIX)
        .then(|| OFFICIAL_PRICING_RULE_NAME.to_owned())
}

fn normalize_model_name(model: &str) -> String {
    let normalized = model.trim().to_ascii_lowercase();
    normalized
        .strip_prefix("openai/")
        .unwrap_or(&normalized)
        .to_owned()
}

fn official_rates(model: &str) -> Option<(&'static str, i64, i64, i64, i64)> {
    let model_name = ["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"]
        .into_iter()
        .find(|candidate| model == *candidate || model.starts_with(&format!("{candidate}-")))?;
    let rates = match model_name {
        "gpt-5.6-sol" => (5_000_000, 500_000, 6_250_000, 30_000_000),
        "gpt-5.6-terra" => (2_500_000, 250_000, 3_125_000, 15_000_000),
        "gpt-5.6-luna" => (1_000_000, 100_000, 1_250_000, 6_000_000),
        _ => return None,
    };
    Some((model_name, rates.0, rates.1, rates.2, rates.3))
}

pub(crate) fn price_with_rules(
    rules: &[PricingRuleRecord],
    usage: &TokenUsage,
    context: &PricingContext<'_>,
) -> PricingOutcome {
    let Some(rule) = rules
        .iter()
        .filter(|rule| rule.active)
        .filter(|rule| rule.effective_from_ms <= context.effective_at_ms)
        .filter(|rule| rule_matches(rule, context))
        .max_by_key(|rule| rule_score(rule))
    else {
        return PricingOutcome::Unpriced {
            rule_id: None,
            reason: "没有匹配的美元价格规则。".into(),
        };
    };
    calculate(rule, usage, context)
}

pub(crate) fn calculate(
    rule: &PricingRuleRecord,
    usage: &TokenUsage,
    _context: &PricingContext<'_>,
) -> PricingOutcome {
    match rule.billing_mode {
        BillingMode::Subscription => PricingOutcome::Subscription {
            rule_id: rule.id.clone(),
            version: rule.version,
        },
        BillingMode::Unpriced => PricingOutcome::Unpriced {
            rule_id: Some(rule.id.clone()),
            reason: "此规则设置为不估算费用。".into(),
        },
        BillingMode::Token => {
            let normal_input = if rule.cache_write_included_in_input {
                usage
                    .input_tokens
                    .saturating_sub(usage.cached_input_tokens)
                    .saturating_sub(usage.cache_write_input_tokens)
            } else {
                usage.input_tokens.saturating_sub(usage.cached_input_tokens)
            };
            let buckets = [
                ("普通输入", normal_input, rule.input_microusd_per_million),
                (
                    "缓存读取",
                    usage.cached_input_tokens,
                    rule.cached_read_microusd_per_million,
                ),
                (
                    "缓存写入",
                    usage.cache_write_input_tokens,
                    rule.cache_write_microusd_per_million,
                ),
                (
                    "输出",
                    usage.output_tokens,
                    rule.output_microusd_per_million,
                ),
            ];
            let mut cost = 0i128;
            for (label, tokens, price) in buckets {
                if tokens == 0 {
                    continue;
                }
                let Some(price) = price else {
                    return PricingOutcome::Unpriced {
                        rule_id: Some(rule.id.clone()),
                        reason: format!("缺少{label}单价。"),
                    };
                };
                if price < 0 {
                    return PricingOutcome::Unpriced {
                        rule_id: Some(rule.id.clone()),
                        reason: format!("{label}单价无效。"),
                    };
                }
                let bucket_cost = (tokens as i128)
                    .checked_mul(price as i128)
                    .and_then(|value| value.checked_div(1_000_000))
                    .and_then(|value| cost.checked_add(value));
                let Some(bucket_cost) = bucket_cost else {
                    return PricingOutcome::Unpriced {
                        rule_id: Some(rule.id.clone()),
                        reason: "费用计算超出整数范围。".into(),
                    };
                };
                cost = bucket_cost;
            }
            if let Some(fee) = rule.request_fee_microusd {
                if fee < 0 {
                    return PricingOutcome::Unpriced {
                        rule_id: Some(rule.id.clone()),
                        reason: "请求固定费无效。".into(),
                    };
                }
                cost = match cost.checked_add(fee as i128) {
                    Some(value) => value,
                    None => {
                        return PricingOutcome::Unpriced {
                            rule_id: Some(rule.id.clone()),
                            reason: "费用计算超出整数范围。".into(),
                        };
                    }
                };
            }
            match i64::try_from(cost) {
                Ok(cost_microusd) => PricingOutcome::Estimated {
                    cost_microusd,
                    rule_id: rule.id.clone(),
                    version: rule.version,
                },
                Err(_) => PricingOutcome::Unpriced {
                    rule_id: Some(rule.id.clone()),
                    reason: "费用计算超出数据库范围。".into(),
                },
            }
        }
    }
}

pub(crate) fn parse_usd_microusd(value: &str) -> Result<i64, String> {
    let value = value.trim();
    if value.is_empty()
        || value.starts_with('-')
        || value.chars().any(|char| matches!(char, 'e' | 'E' | '+'))
    {
        return Err("美元价格必须是非负十进制数字，不能使用科学计数法。".into());
    }
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty() || !whole.chars().all(|char| char.is_ascii_digit()) {
        return Err("美元价格格式不正确。".into());
    }
    if fraction.len() > 6 || !fraction.chars().all(|char| char.is_ascii_digit()) {
        return Err("美元价格最多支持 6 位小数。".into());
    }
    let whole = whole
        .parse::<i128>()
        .map_err(|_| "美元价格超出可保存范围。".to_owned())?;
    let fraction_value = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse::<i128>()
            .map_err(|_| "美元价格格式不正确。".to_owned())?
            * 10i128.pow((6 - fraction.len()) as u32)
    };
    i64::try_from(
        whole
            .checked_mul(1_000_000)
            .and_then(|value| value.checked_add(fraction_value))
            .ok_or_else(|| "美元价格超出可保存范围。".to_owned())?,
    )
    .map_err(|_| "美元价格超出可保存范围。".into())
}

fn rule_matches(rule: &PricingRuleRecord, context: &PricingContext<'_>) -> bool {
    let scope_matches = match rule.scope_kind {
        PricingScopeKind::AccountModel => rule.account_id.as_deref() == context.account_id,
        PricingScopeKind::ProviderModel | PricingScopeKind::ProviderDefault => {
            rule.provider_id.as_deref() == context.provider_id
        }
        PricingScopeKind::GlobalModel => true,
    };
    if !scope_matches {
        return false;
    }
    if rule.scope_kind == PricingScopeKind::ProviderDefault {
        return true;
    }
    match rule.match_kind {
        PricingMatchKind::Exact => rule.model_pattern == context.model,
        PricingMatchKind::Prefix => context.model.starts_with(&rule.model_pattern),
    }
}

fn rule_score(rule: &PricingRuleRecord) -> (u8, u8, usize, i64, i64) {
    let scope = match rule.scope_kind {
        PricingScopeKind::AccountModel => 4,
        PricingScopeKind::ProviderModel => 3,
        PricingScopeKind::GlobalModel => 2,
        PricingScopeKind::ProviderDefault => 1,
    };
    let matching = match rule.match_kind {
        PricingMatchKind::Exact => 2,
        PricingMatchKind::Prefix => 1,
    };
    (
        scope,
        matching,
        rule.model_pattern.len(),
        rule.effective_from_ms,
        rule.version,
    )
}

#[cfg(test)]
mod tests {
    use super::{
        PricingContext, PricingOutcome, PricingRuleRecord, calculate, parse_usd_microusd,
        price_with_official_catalog,
    };
    use crate::{
        models::{BillingMode, PricingMatchKind, PricingScopeKind},
        usage_log::TokenUsage,
    };

    #[test]
    fn prices_official_luna_with_the_builtin_openai_catalog() {
        let result = price_with_official_catalog(
            &TokenUsage {
                input_tokens: 2_690_000,
                cached_input_tokens: 2_450_000,
                cache_write_input_tokens: 0,
                output_tokens: 4_900,
                reasoning_output_tokens: 0,
                total_tokens: 2_694_900,
            },
            &PricingContext {
                model: "gpt-5.6-luna",
                provider_id: None,
                account_id: None,
                effective_at_ms: 1,
            },
        );

        assert_eq!(
            result,
            PricingOutcome::Estimated {
                cost_microusd: 514_400,
                rule_id: "openai-official-standard:gpt-5.6-luna".into(),
                version: 20260801,
            }
        );
    }

    #[test]
    fn unknown_official_models_are_not_assigned_a_guessed_price() {
        let result = price_with_official_catalog(
            &TokenUsage {
                input_tokens: 1,
                cached_input_tokens: 0,
                cache_write_input_tokens: 0,
                output_tokens: 1,
                reasoning_output_tokens: 0,
                total_tokens: 2,
            },
            &PricingContext {
                model: "gpt-9-preview",
                provider_id: None,
                account_id: None,
                effective_at_ms: 1,
            },
        );

        assert!(matches!(result, PricingOutcome::Unpriced { .. }));
    }

    #[test]
    fn calculates_usd_with_separate_cache_buckets() {
        let rule = PricingRuleRecord::token(
            "relay-model",
            PricingScopeKind::ProviderModel,
            Some("relay"),
            1_200_000,
            300_000,
            1_200_000,
            8_000_000,
        );
        let usage = TokenUsage {
            input_tokens: 100,
            cached_input_tokens: 20,
            cache_write_input_tokens: 3,
            output_tokens: 8,
            reasoning_output_tokens: 2,
            total_tokens: 108,
        };

        let result = calculate(
            &rule,
            &usage,
            &PricingContext {
                model: "relay-model",
                provider_id: Some("relay"),
                account_id: None,
                effective_at_ms: 1,
            },
        );

        assert_eq!(
            result,
            PricingOutcome::Estimated {
                cost_microusd: 165,
                rule_id: "relay-model".into(),
                version: 1,
            }
        );
    }

    #[test]
    fn cache_write_can_be_excluded_from_total_input() {
        let mut rule = PricingRuleRecord::token(
            "relay-model",
            PricingScopeKind::GlobalModel,
            None,
            1_000_000,
            0,
            2_000_000,
            0,
        );
        rule.cache_write_included_in_input = false;
        let usage = TokenUsage {
            input_tokens: 10,
            cached_input_tokens: 2,
            cache_write_input_tokens: 3,
            output_tokens: 0,
            reasoning_output_tokens: 0,
            total_tokens: 10,
        };

        let result = calculate(
            &rule,
            &usage,
            &PricingContext {
                model: "relay-model",
                provider_id: None,
                account_id: None,
                effective_at_ms: 1,
            },
        );

        assert_eq!(
            result,
            PricingOutcome::Estimated {
                cost_microusd: 14,
                rule_id: "relay-model".into(),
                version: 1,
            }
        );
    }

    #[test]
    fn subscription_and_missing_price_are_not_zero_cost_estimates() {
        let subscription = PricingRuleRecord {
            billing_mode: BillingMode::Subscription,
            ..PricingRuleRecord::token(
                "subscription-model",
                PricingScopeKind::GlobalModel,
                None,
                0,
                0,
                0,
                0,
            )
        };
        let missing = PricingRuleRecord::token(
            "missing-model",
            PricingScopeKind::GlobalModel,
            None,
            1_000_000,
            0,
            0,
            0,
        );
        let mut missing = missing;
        missing.output_microusd_per_million = None;
        let usage = TokenUsage {
            input_tokens: 1,
            cached_input_tokens: 0,
            cache_write_input_tokens: 0,
            output_tokens: 1,
            reasoning_output_tokens: 0,
            total_tokens: 2,
        };

        assert!(matches!(
            calculate(
                &subscription,
                &usage,
                &PricingContext {
                    model: "subscription-model",
                    provider_id: None,
                    account_id: None,
                    effective_at_ms: 1,
                }
            ),
            PricingOutcome::Subscription { .. }
        ));
        assert!(matches!(
            calculate(
                &missing,
                &usage,
                &PricingContext {
                    model: "missing-model",
                    provider_id: None,
                    account_id: None,
                    effective_at_ms: 1,
                }
            ),
            PricingOutcome::Unpriced { .. }
        ));
    }

    #[test]
    fn parses_only_plain_decimal_usd_values() {
        assert_eq!(parse_usd_microusd("1.20").unwrap(), 1_200_000);
        assert_eq!(parse_usd_microusd("0.000001").unwrap(), 1);
        assert!(parse_usd_microusd("1.2345678").is_err());
        assert!(parse_usd_microusd("1e2").is_err());
        assert!(parse_usd_microusd("-1").is_err());
    }

    #[test]
    fn account_exact_rule_beats_provider_prefix_rule() {
        let provider = PricingRuleRecord::token(
            "provider-prefix",
            PricingScopeKind::ProviderModel,
            Some("relay"),
            1_000_000,
            0,
            0,
            0,
        );
        let mut account = PricingRuleRecord::token(
            "account-exact",
            PricingScopeKind::AccountModel,
            Some("account"),
            2_000_000,
            0,
            0,
            0,
        );
        account.match_kind = PricingMatchKind::Exact;
        let usage = TokenUsage {
            input_tokens: 1,
            cached_input_tokens: 0,
            cache_write_input_tokens: 0,
            output_tokens: 0,
            reasoning_output_tokens: 0,
            total_tokens: 1,
        };

        let result = super::price_with_rules(
            &[provider, account],
            &usage,
            &PricingContext {
                model: "account-exact",
                provider_id: Some("relay"),
                account_id: Some("account"),
                effective_at_ms: 1,
            },
        );

        assert_eq!(
            result,
            PricingOutcome::Estimated {
                cost_microusd: 2,
                rule_id: "account-exact".into(),
                version: 1,
            }
        );
    }
}
