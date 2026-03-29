# Shell Completion

## User Story
As a LoongClaw user on bash, zsh, fish, powershell, or elvish,
I want to generate a shell completion script so that I can
tab-complete subcommands and flags without memorizing them.

## Acceptance Criteria
- [ ] `loong completions bash` prints a bash completion script to stdout.
- [ ] `loong completions zsh` prints a zsh completion script to stdout.
- [ ] `loong completions fish` prints a fish completion script to stdout.
- [ ] `loong completions powershell` prints a PowerShell completion script to stdout.
- [ ] `loong completions elvish` prints an elvish completion script to stdout.
- [ ] Invalid shell names produce a structured clap error and exit code 2.
- [ ] CI generates completion files as release artifacts (`loong.bash`, `_loong`, `loong.fish`, `loong.ps1`, `loong.elv`).

## Install Instructions

### bash

```bash
loong completions bash >> ~/.bash_completion
source ~/.bash_completion
```

### zsh

```zsh
loong completions zsh > "${fpath[1]}/_loong"
# Ensure the directory is in $fpath before compinit runs.
```

### fish

```fish
loong completions fish > ~/.config/fish/completions/loong.fish
```

### PowerShell

```powershell
loong completions powershell >> $PROFILE
```

### elvish

```elvish
loong completions elvish >> ~/.config/elvish/rc.elv
```

## Out of Scope
- Auto-installing completions during `loong onboard`.
