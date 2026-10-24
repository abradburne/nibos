# nibOS: the repository's checks. Building is per piece (docs/building.md);
# this only checks what can be checked without a device.
#
#   make check          everything below
#   make shellcheck     every shell script
#   make boards         every fully described board sets what the scripts read
#   make rust           rustfmt, clippy and unit tests (in the build machine on a Mac)
#   make python         every Python file parses, and mkboot.py's unit tests
#   make links          the Markdown files' relative links

.PHONY: check shellcheck boards rust python links

check:
	tools/check.sh all

shellcheck:
	tools/check.sh shell

boards:
	tools/check.sh boards

rust:
	tools/check.sh rust

python:
	tools/check.sh python

links:
	tools/check.sh links
