#!/usr/bin/env python3
"""Preserve the frozen log filter; add only competition start-refusal reasons."""
import re
import sys
import console


class Progress(console.Progress):
    def line(self, raw):
        refusal = re.fullmatch(rb'\[competition\] start_refused reason=(partial_cluster|endpoint_unset)\r?\n?', raw)
        if refusal:
            return '[run.sh] solver_exited result=start_failed reason=' + refusal[1].decode('ascii')
        return super().line(raw)


def main():
    console.Progress = Progress
    return console.main()


if __name__ == '__main__':
    sys.exit(main())
