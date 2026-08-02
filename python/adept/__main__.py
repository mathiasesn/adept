import os
import sys

from adept import find_adept_bin


def find_adept_bin_and_exec() -> None:
    adept = find_adept_bin()

    if sys.platform == "win32":
        # Windows has no working os.execvp, so spawn and forward the exit
        # code instead. Imported here so the POSIX path, which replaces the
        # process anyway, does not pay for the import on every run.
        import subprocess

        try:
            completed_process = subprocess.run([adept, *sys.argv[1:]])
        except KeyboardInterrupt:
            # 130 (128 + SIGINT), not 2: adept documents 2 as its usage/I-O
            # error code, which would make an interrupt indistinguishable
            # from a bad invocation.
            sys.exit(130)
        sys.exit(completed_process.returncode)
    else:
        os.execvp(adept, [adept, *sys.argv[1:]])


if __name__ == "__main__":
    find_adept_bin_and_exec()
