import os
import subprocess
import sys

from adept import find_adept_bin


def find_adept_bin_and_exec() -> None:
    adept = find_adept_bin()

    if sys.platform == "win32":
        # Windows does not support os.execvp / os.execv properly, so we
        # spawn a subprocess and forward its return code instead. A raised
        # KeyboardInterrupt on Ctrl-C would otherwise print a traceback for
        # something the user intentionally did, so it is caught and turned
        # into a plain exit(2).
        completed_process = subprocess.run([adept, *sys.argv[1:]])
        try:
            sys.exit(completed_process.returncode)
        except KeyboardInterrupt:
            sys.exit(2)
    else:
        os.execvp(adept, [adept, *sys.argv[1:]])


if __name__ == "__main__":
    find_adept_bin_and_exec()
