#!/usr/bin/env python3
import argparse
from datetime import datetime, timedelta
import json
from pathlib import Path
import sys
from typing import Optional


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Query nexus-alert-relay JSONL audit logs."
    )
    parser.add_argument(
        "--path",
        default="/tmp/nexus-alert-relay/audit.jsonl",
        help="Audit JSONL file path or directory containing rotated audit files.",
    )
    parser.add_argument("--channel", help="Filter by channel, e.g. dry-run/generic/feishu/wecom.")
    parser.add_argument("--status", help="Filter by payload status, e.g. firing/resolved.")
    parser.add_argument("--alertname", help="Filter by alertname label.")
    parser.add_argument("--severity", help="Filter by severity label.")
    parser.add_argument(
        "--since",
        help=(
            "Only include records at or after this time. "
            "Supports relative values like 15m/2h/1d, Unix seconds/ms, or ISO-8601."
        ),
    )
    parser.add_argument(
        "--contains",
        help="Match a case-insensitive keyword against text, markdown, and payload JSON.",
    )
    parser.add_argument(
        "--limit",
        type=int,
        default=20,
        help="Maximum number of matched records to print. Use 0 for no limit.",
    )
    parser.add_argument(
        "--format",
        choices=["text", "json"],
        default="text",
        help="Output format.",
    )
    parser.add_argument(
        "--summary",
        choices=["channel", "severity", "alertname"],
        help="Show aggregated counts by this dimension instead of raw records.",
    )
    parser.add_argument(
        "--export",
        help="Write matched records as JSONL to this file.",
    )
    return parser.parse_args()


def parse_since(value: Optional[str]) -> Optional[int]:
    if not value:
        return None

    raw = value.strip()
    if not raw:
        return None

    if raw[-1:] in {"s", "m", "h", "d"} and raw[:-1].isdigit():
        amount = int(raw[:-1])
        unit = raw[-1]
        delta = {
            "s": timedelta(seconds=amount),
            "m": timedelta(minutes=amount),
            "h": timedelta(hours=amount),
            "d": timedelta(days=amount),
        }[unit]
        return int((datetime.now() - delta).timestamp() * 1000)

    if raw.isdigit():
        numeric = int(raw)
        if numeric >= 1_000_000_000_000:
            return numeric
        return numeric * 1000

    normalized = raw.replace("Z", "+00:00")
    try:
        parsed = datetime.fromisoformat(normalized)
    except ValueError as exc:
        raise argparse.ArgumentTypeError(
            f"invalid --since value: {value}. Use 15m/2h/1d, Unix seconds/ms, or ISO-8601."
        ) from exc
    return int(parsed.timestamp() * 1000)


def iter_files(path_value: str):
    path = Path(path_value)
    if path.is_dir():
        yield from sorted(p for p in path.glob("*.jsonl") if p.is_file())
        return

    if path.exists():
        yield path
        parent = path.parent
        stem = path.stem
        suffix = path.suffix
        yield from sorted(
            p
            for p in parent.glob(f"{stem}-*{suffix}")
            if p.is_file()
        )
        return

    raise FileNotFoundError(f"audit path does not exist: {path}")


def matches(record: dict, args: argparse.Namespace) -> bool:
    if args.channel and record.get("channel") != args.channel:
        return False
    if args.status and record.get("status") != args.status:
        return False
    if args.since_ms is not None and int(record.get("ts_ms", 0)) < args.since_ms:
        return False

    alerts = record.get("payload", {}).get("alerts", [])
    if args.alertname:
        if not any(alert.get("labels", {}).get("alertname") == args.alertname for alert in alerts):
            return False
    if args.severity:
        if not any(alert.get("labels", {}).get("severity") == args.severity for alert in alerts):
            return False
    if args.contains:
        haystack = "\n".join(
            [
                record.get("text", ""),
                record.get("markdown", ""),
                json.dumps(record.get("payload", {}), ensure_ascii=False),
            ]
        ).lower()
        if args.contains.lower() not in haystack:
            return False
    return True


def record_summary(record: dict) -> str:
    alerts = record.get("payload", {}).get("alerts", [])
    labels = alerts[0].get("labels", {}) if alerts else {}
    return " | ".join(
        [
            f"ts_ms={record.get('ts_ms', '-')}",
            f"channel={record.get('channel', '-')}",
            f"status={record.get('status', '-')}",
            f"alertname={labels.get('alertname', '-')}",
            f"severity={labels.get('severity', '-')}",
            f"count={record.get('alert_count', 0)}",
            f"text={record.get('text', '').replace(chr(10), ' / ')}",
        ]
    )


def summary_key(record: dict, dimension: str) -> str:
    if dimension == "channel":
        return str(record.get("channel", "-"))

    alerts = record.get("payload", {}).get("alerts", [])
    labels = alerts[0].get("labels", {}) if alerts else {}
    if dimension == "severity":
        return str(labels.get("severity", "-"))
    if dimension == "alertname":
        return str(labels.get("alertname", "-"))
    return "-"


def main() -> int:
    args = parse_args()
    try:
        args.since_ms = parse_since(args.since)
    except argparse.ArgumentTypeError as exc:
        print(str(exc), file=sys.stderr)
        return 2
    matched = []
    try:
        for file_path in iter_files(args.path):
            with file_path.open("r", encoding="utf-8") as fh:
                for raw_line in fh:
                    line = raw_line.strip()
                    if not line:
                        continue
                    try:
                        record = json.loads(line)
                    except json.JSONDecodeError:
                        continue
                    if matches(record, args):
                        matched.append(record)
    except FileNotFoundError as exc:
        print(str(exc), file=sys.stderr)
        return 1

    if args.export:
        export_path = Path(args.export)
        export_path.parent.mkdir(parents=True, exist_ok=True)
        with export_path.open("w", encoding="utf-8") as fh:
            for record in matched:
                fh.write(json.dumps(record, ensure_ascii=False) + "\n")

    if args.summary:
        buckets = {}
        for record in matched:
            key = summary_key(record, args.summary)
            buckets[key] = buckets.get(key, 0) + 1
        ordered = sorted(buckets.items(), key=lambda item: (-item[1], item[0]))
        if args.format == "json":
            print(
                json.dumps(
                    [
                        {"key": key, "count": count, "dimension": args.summary}
                        for key, count in ordered
                    ],
                    ensure_ascii=False,
                    indent=2,
                )
            )
        else:
            if not ordered:
                print("No matching audit records.")
            for key, count in ordered:
                print(f"{args.summary}={key} | count={count}")
        return 0

    if args.limit > 0:
        matched = matched[-args.limit :]

    if args.format == "json":
        print(json.dumps(matched, ensure_ascii=False, indent=2))
    else:
        if not matched:
            print("No matching audit records.")
        for record in matched:
            print(record_summary(record))

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
