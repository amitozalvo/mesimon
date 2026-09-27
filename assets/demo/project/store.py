"""The link table. A dict in memory is enough for a demo."""

from dataclasses import dataclass


@dataclass
class Link:
    slug: str
    target: str
    clicks: int = 0
    expired: bool = False


_LINKS = {
    "docs": Link("docs", "https://example.com/docs", 42),
    "blog": Link("blog", "https://example.com/blog", 7),
}


def links():
    return list(_LINKS.values())
