#!/usr/bin/env python3
import re
import sys
from pathlib import Path


def fail(msg: str) -> None:
    print(f"error: {msg}", file=sys.stderr)
    sys.exit(1)


if len(sys.argv) > 2:
    fail("usage: apply_vertexlauncher_context_delete_dialog_fix_v2.py [repo_root]")

repo = Path(sys.argv[1]).resolve() if len(sys.argv) == 2 else Path.cwd()
app_mod = repo / "crates" / "vertexlauncher" / "src" / "app" / "mod.rs"

if not app_mod.exists():
    fail(f"could not find {app_mod}")

text = app_mod.read_text(encoding="utf-8")

replacement = """InstanceContextAction::Delete => {
            self.selected_instance_id = Some(instance_id.clone());
            self.active_screen = screens::AppScreen::Library;
            screens::request_delete_instance(ctx, &instance_id);
        }"""

patterns = [
    re.compile(
        r"InstanceContextAction::Delete\s*=>\s*\{\s*self\.begin_delete_instance\(\s*&instance_id\s*\);\s*\}",
        re.DOTALL,
    ),
    re.compile(
        r"InstanceContextAction::Delete\s*=>\s*\{\s*self\.(?:delete_instance|delete_instance_by_id|remove_instance|remove_instance_by_id)\(\s*&instance_id\s*\);\s*\}",
        re.DOTALL,
    ),
]

replaced = False
for pattern in patterns:
    new_text, count = pattern.subn(replacement, text, count=1)
    if count:
        text = new_text
        replaced = True
        break

if not replaced:
    generic = re.compile(
        r"InstanceContextAction::Delete\s*=>\s*\{.*?\n\s*\}",
        re.DOTALL,
    )
    new_text, count = generic.subn(replacement, text, count=1)
    if count:
        text = new_text
        replaced = True

if not replaced:
    fail("could not find InstanceContextAction::Delete match arm to patch")

if "screens::request_delete_instance(ctx, &instance_id);" not in text:
    fail("failed to apply delete-dialog patch")

app_mod.write_text(text, encoding="utf-8")
print(f"patched {app_mod}")
