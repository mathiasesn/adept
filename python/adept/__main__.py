import os
import sys

from adept import find_adept_bin


def find_adept_bin_and_exec() -> None:
    adept = find_adept_bin()

    if sys.platform == "win32":
        # Windows does not support os.execvp / os.execv properly, so we
        # spawn a subprocess and forward its return code instead. A raised
        # KeyboardInterrupt on Ctrl-C would otherwise print a traceback for
        # something the user intentionally did, so it is caught and turned
        # into exit(130) (128 + SIGINT), not a plain exit(2): adept's `2`
        # is documented as its usage/I-O-error exit code, so reusing it here
        # would make an interrupt indistinguishable from a bad invocation.
        # This is a deliberate divergence from ruff, on a branch CI never
        # runs. Imported here, not at module scope, so the POSIX path —
        # which is about to replace the process anyway — does not pay to
        # import subprocess and its dependencies on every run.
        import subprocess

        try:
            completed_process = subprocess.run([adept, *sys.argv[1:]])
        except KeyboardInterrupt:
            sys.exit(130)
        sys.exit(completed_process.returncode)
    else:
        os.execvp(adept, [adept, *sys.argv[1:]])


if __name__ == "__main__":
    find_adept_bin_and_exec()
