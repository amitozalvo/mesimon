"""`shortlink stats`: click counts per link, as a table."""

import sys

from store import links


def main(argv=sys.argv[1:]):
    rows = sorted(links(), key=lambda link: -link.clicks)
    print(f"{'slug':<10} {'clicks':>7}  target")
    for link in rows:
        print(f"{link.slug:<10} {link.clicks:>7}  {link.target}")


if __name__ == "__main__":
    main()
