from playwright.sync_api import sync_playwright

from .browser import TikTokNavigator
from .config import AppConfig, BROWSER_LOCALE
from .cookies import load_netscape_cookies
from .downloader import download_video
from .urls import get_video_id


def _print_config(config: AppConfig) -> None:
    print("=" * 70)
    print("TikTok Short Drama Downloader")
    print("=" * 70)
    print(f"URL      : {config.start_url}")
    print(f"Episodes : {config.total_episodes}")
    print(f"Cookies  : {config.cookie_file}")
    print(f"Output   : {config.output_dir}")
    print("=" * 70)


def _print_done() -> None:
    print()
    print("=" * 70)
    print("DONE")
    print("=" * 70)


def run(config: AppConfig) -> int:
    """Run the downloader using the supplied application configuration."""
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
            print("[+] Opening EP1...")
            navigator.open(config.start_url)
            print(f"[+] Opened: {page.url}")

            navigator.try_open_episode_list()

            previous_video_id: str | None = None

            for episode in range(1, config.total_episodes + 1):
                print()
                print(f"[+] EP {episode}")

                video_url = navigator.get_current_video_url()
                if not video_url:
                    print(f"[ERROR] Không lấy được URL EP {episode}")
                    return 1

                current_video_id = get_video_id(video_url)

                print(f"[+] URL: {video_url}")
                print(f"[+] Video ID: {current_video_id}")

                if previous_video_id and current_video_id == previous_video_id:
                    print("[!] WARNING: Video ID không đổi.")
                    print("[!] TikTok có thể chưa chuyển sang tập mới.")

                download_video(
                    url=video_url,
                    episode=episode,
                    cookie_file=config.cookie_file,
                    output_dir=config.output_dir,
                )

                previous_video_id = current_video_id

                if episode == config.total_episodes:
                    break

                print()
                print(f"[+] Scroll EP {episode} -> EP {episode + 1}")
                navigator.go_to_next_episode()
        finally:
            print()
            print("[+] Closing browser...")
            browser.close()

    _print_done()
    return 0
