---
title: completions
description: Generate shell completions for rung commands.
---

Generate shell completion scripts for tab-completion of rung commands and options.

## Usage

```bash
rung completions <shell>
```

## Aliases

- `rung comp` — shorthand for `rung completions`

## Supported Shells

| Shell      | Value        |
| ---------- | ------------ |
| Bash       | `bash`       |
| Zsh        | `zsh`        |
| Fish       | `fish`       |
| PowerShell | `powershell` |
| Elvish     | `elvish`     |

## Installation

### Bash

```bash
# Add to ~/.bashrc or ~/.bash_profile
rung completions bash >> ~/.bash_completion
source ~/.bash_completion
```

Or for system-wide installation:

```bash
rung completions bash | sudo tee /etc/bash_completion.d/rung > /dev/null
```

### Zsh

```bash
# Create completions directory if needed
mkdir -p ~/.zsh/completions

# Generate completions
rung completions zsh > ~/.zsh/completions/_rung

# Add to ~/.zshrc (if not already present)
echo 'fpath=(~/.zsh/completions $fpath)' >> ~/.zshrc
echo 'autoload -Uz compinit && compinit' >> ~/.zshrc
```

### Fish

```bash
rung completions fish > ~/.config/fish/completions/rung.fish
```

### PowerShell

```powershell
# Add to your PowerShell profile
rung completions powershell >> $PROFILE
```

### Elvish

```bash
rung completions elvish > ~/.elvish/lib/rung.elv
# Then add `use rung` to ~/.elvish/rc.elv
```

## Example

After installing completions, you can tab-complete:

```bash
$ rung <TAB>
create   doctor   init     log      merge    move     nxt      prv
status   submit   sync     undo     update   completions

$ rung sync --<TAB>
--abort      --base       --continue   --dry-run    --json       --no-push    --quiet

$ rung merge --method <TAB>
merge    rebase   squash
```

## Notes

- Completions are generated from the CLI definition, so they're always up-to-date
- You may need to restart your shell or source your config after installation
- Some shells require additional setup for completions to work

## Related Commands

- [`doctor`](/commands/doctor/) — Verify rung is working correctly
