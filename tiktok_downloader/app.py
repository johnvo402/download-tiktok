from playwright.sync_api import sync_playwright

from .browser import TikTokNavigator
from .config import AppConfig, BROWSER_LOCALE
from .cookies import load_netscape_cookies
from .downloader import download_video
from .metadata import (
    create_series_directory,
    download_poster,
    write_metadata,
)
from .urls import get_video_id


def _print_config(config: AppConfig) -> None:
    print("=" * 70)
    print("TikTok Short Drama Downloader")
    print("=" * 70)
    print(f"URL      : {config.start_url}")
    print(
        "Episodes : "
        + (
            str(config.total_episodes)
            if config.total_episodes is not None
            else "auto"
        )
    )
    print(f"Cookies  : {config.cookie_file}")
    print(f"Output   : {config.output_dir}")
    print("=" * 70)


def _print_done() -> None:
    print()
    print("=" * 70)
    print("DONE")
    print("=" * 70)


def run(config: AppConfig) -> int:
    _print_config(config)

    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(
            headless=False,
            channel="chrome",
        )

        try:
            context = browser.new_context(locale=BROWSER_LOCALE)
            cookies = load_netscape_cookies(config.cookie_file)

            print(f"[+] Loaded {len(cookies)} cookies")
            context.add_cookies(cookies)

            page = context.new_page()
            navigator = TikTokNavigator(page)

            print()
            print("[+] Opening series...")
            navigator.open(config.start_url)
            navigator.try_open_episode_list()

            info = navigator.get_drama_info()
            series_dir = create_series_directory(
                config.output_dir,
                info.title,
            )

            print(f"[+] Title : {info.title}")
            print(f"[+] Folder: {series_dir}")

            poster_file = download_poster(
                request=context.request,
                poster_url=info.poster_url,
                series_dir=series_dir,
            )

            if poster_file:
                print(f"[+] Poster: {poster_file}")

            episode_numbers = navigator.discover_episode_numbers()

            if not episode_numbers:
                print("[ERROR] Không tìm thấy danh sách tập.")
                return 1

            write_metadata(
                info=info,
                series_dir=series_dir,
                source_url=config.start_url,
                episode_numbers=episode_numbers,
                poster_file=poster_file,
            )

            print(
                f"[+] Episodes found: {len(episode_numbers)} "
                f"({episode_numbers[0]} -> {episode_numbers[-1]})"
            )

            episodes_to_download = episode_numbers

            if config.total_episodes is not None:
                episodes_to_download = episode_numbers[
                    : config.total_episodes
                ]

            previous_video_id: str | None = None

            for episode in episodes_to_download:
                print()
                print(f"[+] EP {episode}")

                if not navigator.select_episode(episode):
                    print(
                        f"[ERROR] Không click được button EP {episode}"
                    )
                    return 1

                video_url = navigator.get_current_video_url()
                if not video_url:
                    print(
                        f"[ERROR] Không lấy được URL EP {episode}"
                    )
                    return 1

                current_video_id = get_video_id(video_url)

                print(f"[+] URL: {video_url}")
                print(f"[+] Video ID: {current_video_id}")

                if (
                    previous_video_id
                    and current_video_id == previous_video_id
                ):
                    print(
                        "[ERROR] Video ID không đổi sau khi click tập mới."
                    )
                    return 1

                if not download_video(
                    url=video_url,
                    episode=episode,
                    cookie_file=config.cookie_file,
                    output_dir=series_dir,
                ):
                    return 1

                previous_video_id = current_video_id
        finally:
            print()
            print("[+] Closing browser...")
            browser.close()

    _print_done()
    return 0
