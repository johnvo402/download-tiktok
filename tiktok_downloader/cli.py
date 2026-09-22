import argparse
from pathlib import Path
from typing import Sequence

from .app import run
from .config import (
    AppConfig,
    DEFAULT_COOKIE_FILE,
    DEFAULT_OUTPUT_DIR,
    DEFAULT_TOTAL_EPISODES,
)
from .urls import normalize_url


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="TikTok Short Drama Downloader"
    )
    parser.add_argument(
        "url",
        help="URL của tập đầu tiên",
    )
    parser.add_argument(
        "--episodes",
        type=int,
        default=DEFAULT_TOTAL_EPISODES,
        help=f"Tổng số tập (default: {DEFAULT_TOTAL_EPISODES})",
    )
    parser.add_argument(
        "--cookies",
        default=str(DEFAULT_COOKIE_FILE),
        help=f"File cookies.txt (default: {DEFAULT_COOKIE_FILE})",
    )
    parser.add_argument(
        "--output",
        default=str(DEFAULT_OUTPUT_DIR),
        help=f"Thư mục download (default: {DEFAULT_OUTPUT_DIR})",
    )
    return parser


def parse_config(argv: Sequence[str] | None = None) -> AppConfig:
    args = build_parser().parse_args(argv)

    if args.episodes <= 0:
        raise ValueError("--episodes phải lớn hơn 0")

    return AppConfig(
        start_url=normalize_url(args.url),
        total_episodes=args.episodes,
        cookie_file=Path(args.cookies),
        output_dir=Path(args.output),
    )


def main(argv: Sequence[str] | None = None) -> int:
    try:
        config = parse_config(argv)
    except ValueError as exc:
        print(f"[ERROR] {exc}")
        return 2

    if not config.cookie_file.exists():
        print(f"[ERROR] Không tìm thấy cookie file: {config.cookie_file}")
        return 2

    return run(config)
