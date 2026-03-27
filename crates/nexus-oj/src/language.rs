use std::{collections::HashMap, sync::Arc};

use serde::Serialize;

use crate::JudgeMode;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct LanguageKey(pub String);

impl LanguageKey {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LanguageDescriptor {
    pub key: String,
    pub display_name: &'static str,
    pub source_extension: &'static str,
    pub runtime_family: &'static str,
    pub time_multiplier: u32,
    pub memory_multiplier: u32,
    pub sandbox_profile: SandboxProfile,
    pub seccomp_policy: SeccompPolicy,
    pub supported_modes: Vec<JudgeMode>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodeTemplateRequest {
    pub language: String,
    pub judge_mode: JudgeMode,
    pub template: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxProfile {
    Nsjail,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeccompPolicy {
    CppDefault,
    PythonDefault,
}

pub trait LanguageSpec: Send + Sync {
    fn key(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn source_extension(&self) -> &'static str;
    fn runtime_family(&self) -> &'static str;
    fn time_multiplier(&self) -> u32;
    fn memory_multiplier(&self) -> u32;
    fn sandbox_profile(&self) -> SandboxProfile;
    fn seccomp_policy(&self) -> SeccompPolicy;
    fn supported_modes(&self) -> Vec<JudgeMode>;
    fn submission_template(&self, mode: &JudgeMode) -> String;

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            key: self.key().to_owned(),
            display_name: self.display_name(),
            source_extension: self.source_extension(),
            runtime_family: self.runtime_family(),
            time_multiplier: self.time_multiplier(),
            memory_multiplier: self.memory_multiplier(),
            sandbox_profile: self.sandbox_profile(),
            seccomp_policy: self.seccomp_policy(),
            supported_modes: self.supported_modes(),
        }
    }
}

pub struct LanguageCatalog {
    specs: HashMap<String, Arc<dyn LanguageSpec>>,
}

impl LanguageCatalog {
    pub fn new(specs: Vec<Arc<dyn LanguageSpec>>) -> Self {
        let specs = specs
            .into_iter()
            .map(|spec| (spec.key().to_owned(), spec))
            .collect();
        Self { specs }
    }

    pub fn descriptors(&self) -> Vec<LanguageDescriptor> {
        let mut items: Vec<_> = self.specs.values().map(|spec| spec.descriptor()).collect();
        items.sort_by(|left, right| left.key.cmp(&right.key));
        items
    }

    pub fn template_for(&self, language: &str, mode: &JudgeMode) -> Option<String> {
        self.specs
            .get(language)
            .map(|spec| spec.submission_template(mode))
    }
}

pub fn build_default_catalog() -> Arc<LanguageCatalog> {
    Arc::new(LanguageCatalog::new(vec![
        Arc::new(CppLanguage),
        Arc::new(PythonLanguage),
        Arc::new(RustLanguage),
    ]))
}

struct CppLanguage;

impl LanguageSpec for CppLanguage {
    fn key(&self) -> &'static str {
        "cpp"
    }

    fn display_name(&self) -> &'static str {
        "C++"
    }

    fn source_extension(&self) -> &'static str {
        "cpp"
    }

    fn runtime_family(&self) -> &'static str {
        "native"
    }

    fn time_multiplier(&self) -> u32 {
        1
    }

    fn memory_multiplier(&self) -> u32 {
        1
    }

    fn sandbox_profile(&self) -> SandboxProfile {
        SandboxProfile::Nsjail
    }

    fn seccomp_policy(&self) -> SeccompPolicy {
        SeccompPolicy::CppDefault
    }

    fn supported_modes(&self) -> Vec<JudgeMode> {
        vec![JudgeMode::Acm, JudgeMode::Functional]
    }

    fn submission_template(&self, mode: &JudgeMode) -> String {
        match mode {
            JudgeMode::Acm => r#"#include <bits/stdc++.h>
using namespace std;

int main() {
    ios::sync_with_stdio(false);
    cin.tie(nullptr);

    return 0;
}
"#
            .to_owned(),
            JudgeMode::Functional => r#"#include <bits/stdc++.h>
using namespace std;

class Solution {
public:
    int solve() {
        return 0;
    }
};
"#
            .to_owned(),
            JudgeMode::EasyJudge => String::new(),
        }
    }
}

struct PythonLanguage;

impl LanguageSpec for PythonLanguage {
    fn key(&self) -> &'static str {
        "python"
    }

    fn display_name(&self) -> &'static str {
        "Python 3"
    }

    fn source_extension(&self) -> &'static str {
        "py"
    }

    fn runtime_family(&self) -> &'static str {
        "python"
    }

    fn time_multiplier(&self) -> u32 {
        2
    }

    fn memory_multiplier(&self) -> u32 {
        2
    }

    fn sandbox_profile(&self) -> SandboxProfile {
        SandboxProfile::Nsjail
    }

    fn seccomp_policy(&self) -> SeccompPolicy {
        SeccompPolicy::PythonDefault
    }

    fn supported_modes(&self) -> Vec<JudgeMode> {
        vec![JudgeMode::Acm, JudgeMode::Functional]
    }

    fn submission_template(&self, mode: &JudgeMode) -> String {
        match mode {
            JudgeMode::Acm => r#"def main() -> None:
    pass


if __name__ == "__main__":
    main()
"#
            .to_owned(),
            JudgeMode::Functional => r#"class Solution:
    def solve(self) -> int:
        return 0
"#
            .to_owned(),
            JudgeMode::EasyJudge => String::new(),
        }
    }
}

struct RustLanguage;

impl LanguageSpec for RustLanguage {
    fn key(&self) -> &'static str {
        "rust"
    }

    fn display_name(&self) -> &'static str {
        "Rust"
    }

    fn source_extension(&self) -> &'static str {
        "rs"
    }

    fn runtime_family(&self) -> &'static str {
        "native"
    }

    fn time_multiplier(&self) -> u32 {
        1
    }

    fn memory_multiplier(&self) -> u32 {
        1
    }

    fn sandbox_profile(&self) -> SandboxProfile {
        SandboxProfile::Nsjail
    }

    fn seccomp_policy(&self) -> SeccompPolicy {
        SeccompPolicy::CppDefault
    }

    fn supported_modes(&self) -> Vec<JudgeMode> {
        vec![JudgeMode::Acm, JudgeMode::Functional]
    }

    fn submission_template(&self, mode: &JudgeMode) -> String {
        match mode {
            JudgeMode::Acm => r#"use std::io::{self, Read};

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
}
"#
            .to_owned(),
            JudgeMode::Functional => r#"struct Solution;

impl Solution {
    fn solve(&self) -> i32 {
        0
    }
}
"#
            .to_owned(),
            JudgeMode::EasyJudge => String::new(),
        }
    }
}
