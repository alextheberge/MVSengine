#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Renders the mvs-action shadow-mode PR comment from `lint --advisory
--format json` output and, when available, `report --format json` output
comparing the PR's base manifest to its head manifest.

Usage: render_comment.py <shadow.json> [report.json]
"""
import json
import sys

MARKER = "<!-- mvs-shadow-report -->"


def load(path):
    try:
        with open(path) as handle:
            return json.load(handle)
    except (OSError, json.JSONDecodeError):
        return None


def render(shadow, report):
    lines = [MARKER, "### MVS shadow report", ""]

    status = shadow.get("status", "unknown")
    shadow_summary = shadow.get("shadow")

    if status == "advisory_clean":
        lines.append(
            "Manifest evidence already matches the current code. No axis change needed."
        )
    elif status == "advisory_drift" and shadow_summary:
        lines.append(
            f"**MVS would require `{shadow_summary['projected_identity']}`** "
            f"(currently `{shadow_summary['current_identity']}`)."
        )
        lines.append("")
        axes = []
        if shadow_summary.get("would_require_feat"):
            axes.append("FEAT")
        if shadow_summary.get("would_require_prot"):
            axes.append("PROT")
        if axes:
            lines.append(f"Axis: **{', '.join(axes)}**")
        lines.append("")
        failures = shadow.get("failures") or []
        if failures:
            lines.append("<details><summary>Details</summary>")
            lines.append("")
            for failure in failures:
                lines.append(f"- {failure}")
            lines.append("")
            lines.append("</details>")
    else:
        lines.append(f"Status: `{status}` (see the workflow log for details).")

    if report and report.get("status") == "changed":
        lines.append("")
        lines.append("---")
        lines.append("")
        sections = ", ".join(report.get("changed_sections", [])) or "none"
        lines.append(
            f"Manifest changed vs. the base branch: **{report.get('change_count', 0)} "
            f"change(s)** across {sections}."
        )
        identity = report.get("comparison", {}).get("identity")
        if identity:
            lines.append(
                f"Identity: `{identity.get('base')}` -> `{identity.get('target')}` "
                f"(ARCH {identity.get('arch_delta', 0):+d}, "
                f"FEAT {identity.get('feat_delta', 0):+d}, "
                f"PROT {identity.get('prot_delta', 0):+d})"
            )

    lines.append("")
    lines.append(
        "_This is a non-blocking shadow-mode report from "
        "[mvs-manager](https://github.com/alextheberge/MVSengine); it never fails the build._"
    )
    return "\n".join(lines) + "\n"


def main():
    if len(sys.argv) < 2:
        print("usage: render_comment.py <shadow.json> [report.json]", file=sys.stderr)
        sys.exit(2)

    shadow = load(sys.argv[1])
    if shadow is None:
        print(f"{MARKER}\n### MVS shadow report\n\nNo shadow-mode output was produced.\n")
        return

    report = load(sys.argv[2]) if len(sys.argv) > 2 else None
    print(render(shadow, report), end="")


if __name__ == "__main__":
    main()
