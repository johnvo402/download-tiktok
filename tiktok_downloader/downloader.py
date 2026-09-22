import subprocess
import sys
from pathlib import Path


def build_download_command(
    url: str,
    episode: int,
    cookie_file: Path,
    output_dir: Path,
) -> list[str]:
    output_template = output_dir / f"EP{episode:04d}.%(ext)s"

    return [
        sys.executable,
        "-m",
        "yt_dlp",
        "--cookies",
        str(cookie_file),
        "--impersonate",
        "chrome",
        "--no-playlist",
        "-o",
        str(output_template),
        url,
    ]


def download_video(
    url: str,
    episode: int,
    cookie_file: Path,
    output_dir: Path,
) -> bool:
    """Download one episode with yt-dlp."""
    output_dir.mkdir(parents=True, exist_ok=True)

    print()
    print("=" * 70)
    print(f"DOWNLOAD EP {episode:04d}")
    print(url)
    print("=" * 70)

    result = subprocess.run(
        build_download_command(
            url=url,
            episode=episode,
            cookie_file=cookie_file,
            output_dir=output_dir,
        ),
        check=False,
    )

    if result.returncode != 0:
        print(f"[!] Download EP {episode:04d} failed.")
        return False

    return True
