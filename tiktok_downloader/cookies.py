from pathlib import Path
from typing import Any


def load_netscape_cookies(path: Path) -> list[dict[str, Any]]:
    """Load cookies exported in Netscape cookie-file format."""
    cookies: list[dict[str, Any]] = []

    with path.open("r", encoding="utf-8") as cookie_file:
        for raw_line in cookie_file:
            line = raw_line.strip()

            if not line or line.startswith("#"):
                continue

            parts = line.split("\t")
            if len(parts) != 7:
                continue

            domain, _, cookie_path, secure, expires, name, value = parts

            cookie: dict[str, Any] = {
                "name": name,
                "value": value,
                "domain": domain,
                "path": cookie_path,
                "secure": secure.upper() == "TRUE",
            }

            try:
                expires_at = int(expires)
            except ValueError:
                expires_at = 0

            if expires_at > 0:
                cookie["expires"] = expires_at

            cookies.append(cookie)

    return cookies
