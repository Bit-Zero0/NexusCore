use nexus_runtime::{
    debug_detected_runtime_syscall_flavor, debug_seccomp_profile_group_names,
    debug_seccomp_profile_normalized_syscalls_for_flavor,
    debug_seccomp_profile_syscalls_for_flavor, RuntimeSyscallFlavor,
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
    groups: Vec<&'static str>,
    syscalls: Vec<&'static str>,
    normalized: Vec<&'static str>,
}

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let json = args.iter().any(|arg| arg == "--json");
    let flavor = args
        .iter()
        .find_map(|arg| arg.strip_prefix("--flavor="))
        .map(parse_runtime_syscall_flavor)
        .unwrap_or_else(debug_detected_runtime_syscall_flavor);
    let requested = args
        .iter()
        .filter(|arg| arg.as_str() != "--json" && !arg.starts_with("--flavor="))
        .map(|arg| arg.as_str())
        .collect::<Vec<_>>();

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
                flavor,
                groups: debug_seccomp_profile_group_names(policy),
                syscalls: debug_seccomp_profile_syscalls_for_flavor(policy, flavor),
                normalized: debug_seccomp_profile_normalized_syscalls_for_flavor(policy, flavor),
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
        _ => debug_detected_runtime_syscall_flavor(),
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
