---
name: deploy-eink
description: Commit eink-bridge changes, bump the nix_dots flake input, and apply via nixos-rebuild. Use after editing skills, harness files, or server code in the eink-bridge repo.
---

# Deploy eink-bridge

Commit local changes in `eink-bridge`, update the `nix_dots` flake lock, and apply the new system configuration.

## Steps

1. **Commit eink-bridge changes:**
```bash
cd /home/flakm/programming/flakm/eink-bridge
git add -p   # stage interactively, or stage specific files
git commit -m "<message>"
```

2. **Bump the flake input in nix_dots:**
```bash
cd /home/flakm/programming/flakm/nix_dots
nix flake update eink-bridge
```

3. **Commit the lock update:**
```bash
git add flake.lock
git commit -m "bump eink-bridge: <short reason>"
```

4. **Apply:**
```bash
sudo nixos-rebuild switch --flake /home/flakm/programming/flakm/nix_dots#amd-pc
```

5. **Verify** the installed skill updated:
```bash
head -5 /home/flakm/.claude/skills/eink/SKILL.md
```

## Notes

- `nix_dots` home-manager is wired into `nixos-rebuild switch` — no separate `home-manager switch` needed.
- Skills from the harness are deployed as read-only symlinks into `~/.claude/skills/`.
- If only server code changed (not harness/skills), also run `just deploy` in `eink-bridge` to restart the service.
