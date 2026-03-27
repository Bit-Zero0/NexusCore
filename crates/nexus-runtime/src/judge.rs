use std::cmp::Ordering;

use crate::{RuntimeJudgeConfig, RuntimeJudgeMethod, RuntimeValidatorConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseJudgeStatus {
    Accepted,
    WrongAnswer,
}

pub fn validate_output(
    user_output: &str,
    expected_output: &str,
    judge_config: Option<&RuntimeJudgeConfig>,
) -> CaseJudgeStatus {
    let config = effective_validator_config(judge_config);
    if compare_outputs(user_output, expected_output, &config) {
        CaseJudgeStatus::Accepted
    } else {
        CaseJudgeStatus::WrongAnswer
    }
}

fn effective_validator_config(config: Option<&RuntimeJudgeConfig>) -> RuntimeValidatorConfig {
    match config {
        Some(config) if matches!(config.judge_method, RuntimeJudgeMethod::Validator) => config
            .validator
            .clone()
            .unwrap_or_else(default_validator_config),
        _ => default_validator_config(),
    }
}

fn default_validator_config() -> RuntimeValidatorConfig {
    RuntimeValidatorConfig {
        ignore_whitespace: true,
        ignore_case: false,
        is_unordered: false,
        is_token_mode: false,
        is_float: false,
        float_epsilon: 0.0,
    }
}

fn compare_outputs(
    user_output: &str,
    expected_output: &str,
    config: &RuntimeValidatorConfig,
) -> bool {
    if !config.ignore_whitespace
        && !config.is_float
        && !config.is_unordered
        && !config.is_token_mode
    {
        if config.ignore_case {
            return user_output.to_lowercase() == expected_output.to_lowercase();
        }
        return user_output == expected_output;
    }

    let mut user_tokens = tokenize(user_output);
    let mut expected_tokens = tokenize(expected_output);

    if config.ignore_case {
        user_tokens
            .iter_mut()
            .for_each(|token| *token = token.to_lowercase());
        expected_tokens
            .iter_mut()
            .for_each(|token| *token = token.to_lowercase());
    }

    if config.is_float {
        return compare_float_tokens(&user_tokens, &expected_tokens, config);
    }

    if config.is_unordered {
        user_tokens.sort();
        expected_tokens.sort();
    }

    user_tokens == expected_tokens
}

fn tokenize(input: &str) -> Vec<String> {
    input.split_whitespace().map(str::to_owned).collect()
}

fn compare_float_tokens(
    user_tokens: &[String],
    expected_tokens: &[String],
    config: &RuntimeValidatorConfig,
) -> bool {
    if user_tokens.len() != expected_tokens.len() {
        return false;
    }

    let mut user_numbers = match parse_floats(user_tokens) {
        Some(values) => values,
        None => return false,
    };
    let mut expected_numbers = match parse_floats(expected_tokens) {
        Some(values) => values,
        None => return false,
    };

    if config.is_unordered {
        user_numbers.sort_by(float_ordering);
        expected_numbers.sort_by(float_ordering);
    }

    user_numbers
        .iter()
        .zip(expected_numbers.iter())
        .all(|(lhs, rhs)| (lhs - rhs).abs() <= config.float_epsilon)
}

fn parse_floats(tokens: &[String]) -> Option<Vec<f64>> {
    tokens
        .iter()
        .map(|token| token.parse::<f64>().ok())
        .collect::<Option<Vec<_>>>()
}

fn float_ordering(lhs: &f64, rhs: &f64) -> Ordering {
    lhs.partial_cmp(rhs).unwrap_or(Ordering::Equal)
}

#[cfg(test)]
mod tests {
    use super::{validate_output, CaseJudgeStatus};
    use crate::{RuntimeJudgeConfig, RuntimeJudgeMethod, RuntimeValidatorConfig};

    #[test]
    fn default_validator_ignores_whitespace() {
        let status = validate_output("1  2\n3\n", "1 2 3", None);
        assert_eq!(status, CaseJudgeStatus::Accepted);
    }

    #[test]
    fn strict_mode_rejects_format_difference() {
        let status = validate_output(
            "Hello\n",
            "hello\n",
            Some(&judge_config(RuntimeValidatorConfig {
                ignore_whitespace: false,
                ignore_case: false,
                is_unordered: false,
                is_token_mode: false,
                is_float: false,
                float_epsilon: 0.0,
            })),
        );
        assert_eq!(status, CaseJudgeStatus::WrongAnswer);
    }

    #[test]
    fn case_insensitive_validator_accepts_case_difference() {
        let status = validate_output(
            "Hello World",
            "hello world",
            Some(&judge_config(RuntimeValidatorConfig {
                ignore_whitespace: true,
                ignore_case: true,
                is_unordered: false,
                is_token_mode: false,
                is_float: false,
                float_epsilon: 0.0,
            })),
        );
        assert_eq!(status, CaseJudgeStatus::Accepted);
    }

    #[test]
    fn unordered_float_validator_accepts_permuted_values_in_epsilon() {
        let status = validate_output(
            "3.001 1.0 2.0",
            "1.0 2.0 3.0",
            Some(&judge_config(RuntimeValidatorConfig {
                ignore_whitespace: true,
                ignore_case: false,
                is_unordered: true,
                is_token_mode: false,
                is_float: true,
                float_epsilon: 0.01,
            })),
        );
        assert_eq!(status, CaseJudgeStatus::Accepted);
    }

    #[test]
    fn float_validator_rejects_non_numeric_tokens() {
        let status = validate_output(
            "1.0 abc",
            "1.0 2.0",
            Some(&judge_config(RuntimeValidatorConfig {
                ignore_whitespace: true,
                ignore_case: false,
                is_unordered: false,
                is_token_mode: false,
                is_float: true,
                float_epsilon: 0.1,
            })),
        );
        assert_eq!(status, CaseJudgeStatus::WrongAnswer);
    }

    fn judge_config(validator: RuntimeValidatorConfig) -> RuntimeJudgeConfig {
        RuntimeJudgeConfig {
            judge_method: RuntimeJudgeMethod::Validator,
            validator: Some(validator),
            spj: None,
            function_signature: None,
        }
    }
}
