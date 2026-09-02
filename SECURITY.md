# Security Policy

## Supported versions

Vertion is developed on `main`. Fixes land in the next release; there are no
long-term support branches.

| Version | Supported |
| ------- | --------- |
| 1.0.x   | Yes       |
| < 1.0   | No        |

The VSCode extension shares this policy and its version line with the CLI.

## Reporting a vulnerability

Please report privately through GitHub, not in a public issue:

**[Open a private advisory](https://github.com/vertX-dev/vertion/security/advisories/new)**
— or go to the repository's **Security** tab and choose *Report a vulnerability*.

Useful things to include: the version (`vertion --version`), your OS, a minimal
`vertion.cfg` plus source tree that reproduces it, and what you expected to
happen instead.

Expect an acknowledgement within a week. If a report is confirmed, the fix and
an advisory go out together, and you are credited unless you ask otherwise.

## By design: Vertion executes commands from its config file

This is the most important thing to understand before filing a report.

`vertion.cfg` is executable configuration, in the same way `package.json`
scripts and a `Makefile` are. Three fields spawn a shell:

- `run` — post-build commands, executed in the output folder
- `run_here` — post-build commands, executed in the invocation directory
- `[conditions.NAME].cmd` — probes whose exit status gates `[tag{cond}]` markers

They run through `cmd /C` on Windows and `sh -c` elsewhere. So:

> **Cloning an untrusted repository and running `vertion build` runs whatever
> that repository's `vertion.cfg` says.** `vertion condition --list` and
> `vertion condition --hooks` also resolve `cmd` probes, so they execute code
> too, even though neither builds anything.

**Treat a `vertion.cfg` from someone else exactly as you would treat their
`Makefile`: read it before you run it.**

Reports that a config file can run commands describe intended behaviour and
will be closed. What *is* a vulnerability:

- executing anything from a source file's **markers** — markers are data and
  must never reach a shell
- escaping the configured `output` directory when writing a build tree
- running a command the config did not ask for, or in the wrong working
  directory
- a command running when the user explicitly disabled it

## Scope notes for the VSCode extension

`vertion.executablePath` is declared with `"scope": "machine"` and listed in
`capabilities.untrustedWorkspaces.restrictedConfigurations`, so a workspace
cannot redirect the extension at a different binary. If you find a way to make
the extension launch an executable chosen by workspace content, that is a
vulnerability — please report it.

The extension never runs `vertion build`, and so never triggers the `run`,
`run_here`, or `cmd` fields above. It invokes only `vertion map`, which reads
the build manifest and re-filters source files; the conditions it uses are
already-resolved booleans recorded at build time, not probes it re-executes.
