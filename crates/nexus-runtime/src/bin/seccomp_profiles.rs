use nexus_runtime::{
    debug_detected_runtime_syscall_profile, debug_seccomp_profile_group_names,
    debug_seccomp_profile_normalized_syscalls_for_target,
    debug_seccomp_profile_syscalls_for_target, RuntimeSyscallArch, RuntimeSyscallFlavor,
};
use serde::Serialize;
use std::collections::BTreeMap;

const DEFAULT_POLICIES: &[&str] = &[
    "cpp_native_default",
    "rust_native_default",
    "python_default",
    "wasm_default",
    "compiler",
];

#[derive(Debug, Serialize)]
struct SeccompProfileDebugView {
    flavor: RuntimeSyscallFlavor,
    arch: RuntimeSyscallArch,
    groups: Vec<&'static str>,
    syscalls: Vec<&'static str>,
    normalized: Vec<&'static str>,
}

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let json = args.iter().any(|arg| arg == "--json");
    let detected = debug_detected_runtime_syscall_profile();
    let flavor = args
        .iter()
        .find_map(|arg| arg.strip_prefix("--flavor="))
        .map(parse_runtime_syscall_flavor)
        .unwrap_or(detected.flavor);
    let arch = args
        .iter()
        .find_map(|arg| arg.strip_prefix("--arch="))
        .map(parse_runtime_syscall_arch)
        .unwrap_or(detected.arch);
    let requested = args
        .iter()
        .filter(|arg| arg.as_str() != "--json" && !arg.starts_with("--flavor="))
        .map(|arg| arg.as_str())
        .collect::<Vec<_>>();

    let resolved_flavor = if matches!(flavor, RuntimeSyscallFlavor::Auto) {
        detected.flavor
    } else {
        flavor
    };
    let resolved_arch = if matches!(arch, RuntimeSyscallArch::Auto) {
        detected.arch
    } else {
        arch
    };

    let policies = if requested.is_empty() || requested == ["all"] {
        DEFAULT_POLICIES.to_vec()
    } else {
        requested
    };

    let mut views = BTreeMap::new();
    for policy in policies {
        views.insert(
            policy,
            SeccompProfileDebugView {
                flavor: resolved_flavor,
                arch: resolved_arch,
                groups: debug_seccomp_profile_group_names(policy),
                syscalls: debug_seccomp_profile_syscalls_for_target(
                    policy,
                    resolved_flavor,
                    resolved_arch,
                ),
                normalized: debug_seccomp_profile_normalized_syscalls_for_target(
                    policy,
                    resolved_flavor,
                    resolved_arch,
                ),
            },
        );
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&views).expect("serialize seccomp profiles")
        );
        return;
    }

    for (policy, view) in views {
        println!("[{}]", policy);
        println!("flavor:    {}", format_flavor(view.flavor));
        println!("arch:      {}", format_arch(view.arch));
        println!("groups:    {}", view.groups.join(" "));
        println!("syscalls:  {}", view.syscalls.join(" "));
        println!("normalized:{}", view.normalized.join(" "));
        println!();
    }
}

fn parse_runtime_syscall_flavor(value: &str) -> RuntimeSyscallFlavor {
    match value {
        "auto" => RuntimeSyscallFlavor::Auto,
        "generic" => RuntimeSyscallFlavor::Generic,
        "debian_ubuntu" => RuntimeSyscallFlavor::DebianUbuntu,
        "arch" => RuntimeSyscallFlavor::Arch,
        "rhel_like" => RuntimeSyscallFlavor::RhelLike,
        _ => debug_detected_runtime_syscall_profile().flavor,
    }
}

fn parse_runtime_syscall_arch(value: &str) -> RuntimeSyscallArch {
    match value {
        "auto" => RuntimeSyscallArch::Auto,
        "x86_64" => RuntimeSyscallArch::X86_64,
        "aarch64" => RuntimeSyscallArch::Aarch64,
        "other" => RuntimeSyscallArch::Other,
        _ => debug_detected_runtime_syscall_profile().arch,
    }
}

fn format_flavor(flavor: RuntimeSyscallFlavor) -> &'static str {
    match flavor {
        RuntimeSyscallFlavor::Auto => "auto",
        RuntimeSyscallFlavor::Generic => "generic",
        RuntimeSyscallFlavor::DebianUbuntu => "debian_ubuntu",
        RuntimeSyscallFlavor::Arch => "arch",
        RuntimeSyscallFlavor::RhelLike => "rhel_like",
    }
}

fn format_arch(arch: RuntimeSyscallArch) -> &'static str {
    match arch {
        RuntimeSyscallArch::Auto => "auto",
        RuntimeSyscallArch::X86_64 => "x86_64",
        RuntimeSyscallArch::Aarch64 => "aarch64",
        RuntimeSyscallArch::Other => "other",
    }
}
