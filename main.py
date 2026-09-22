import argparse
import re
import subprocess
import sys
import time
from pathlib import Path

from playwright.sync_api import sync_playwright


COOKIE_FILE = "cookies.txt"
OUTPUT_DIR = Path("downloads")
TOTAL_EPISODES = 82


def get_video_id(url: str):
    m = re.search(r"/video/(\d+)", url)
    return m.group(1) if m else None


def go_next_episode(page):
    """
    Lăn chuột xuống để chuyển sang tập kế tiếp.
    """

    # Đưa chuột vào vùng video,
    # tránh hover panel danh sách tập bên phải.
    viewport = page.viewport_size

    if viewport:
        page.mouse.move(
            viewport["width"] * 0.45,
            viewport["height"] * 0.5,
        )

    # Lăn xuống
    page.mouse.wheel(0, 800)

    # Chờ animation + video mới load
    page.wait_for_timeout(1800)


def load_netscape_cookies(path):
    cookies = []

    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()

            if not line or line.startswith("#"):
                continue

            parts = line.split("\t")

            if len(parts) != 7:
                continue

            (
                domain,
                _,
                cookie_path,
                secure,
                expires,
                name,
                value,
            ) = parts

            cookie = {
                "name": name,
                "value": value,
                "domain": domain,
                "path": cookie_path,
                "secure": secure.upper() == "TRUE",
            }

            try:
                exp = int(expires)

                if exp > 0:
                    cookie["expires"] = exp

            except ValueError:
                pass

            cookies.append(cookie)

    return cookies


def find_episode_button(page, episode):
    """
    Tìm ô tập nằm phía bên phải màn hình.
    Tránh nhầm số lượt like/comment.

    Hiện tại loop chính dùng scroll nên hàm này
    chưa cần dùng, nhưng giữ lại nếu cần fallback.
    """

    matches = page.get_by_text(
        str(episode),
        exact=True,
    )

    count = matches.count()

    width = page.evaluate(
        "window.innerWidth"
    )

    for i in range(count):
        item = matches.nth(i)

        try:
            if not item.is_visible():
                continue

            box = item.bounding_box()

            if not box:
                continue

            # Danh sách tập nằm bên phải
            if box["x"] > width * 0.65:
                return item

        except Exception:
            continue

    return None


def get_current_video_url(page):
    """
    Ưu tiên URL hiện tại nếu TikTok
    pushState sang video khác.
    """

    url = page.url

    if re.search(r"/video/\d+", url):
        return url.split("?")[0]

    # Thử tìm link video trong DOM
    links = page.locator(
        'a[href*="/video/"]'
    )

    for i in range(links.count()):
        try:
            href = links.nth(i).get_attribute(
                "href"
            )

            if (
                href
                and re.search(
                    r"/video/\d+",
                    href,
                )
            ):
                if href.startswith("/"):
                    href = (
                        "https://www.tiktok.com"
                        + href
                    )

                return href.split("?")[0]

        except Exception:
            pass

    return None


def download_video(
    url,
    episode,
    cookie_file,
    output_dir,
):
    output_dir.mkdir(
        parents=True,
        exist_ok=True,
    )

    output = str(
        output_dir
        / f"EP{episode:04d}.%(ext)s"
    )

    cmd = [
        sys.executable,
        "-m",
        "yt_dlp",

        "--cookies",
        cookie_file,

        "--impersonate",
        "chrome",

        "--no-playlist",

        "-o",
        output,

        url,
    ]

    print()
    print("=" * 70)
    print(
        f"DOWNLOAD EP {episode:04d}"
    )
    print(url)
    print("=" * 70)

    result = subprocess.run(
        cmd,
        check=False,
    )

    if result.returncode != 0:
        print(
            f"[!] Download EP "
            f"{episode:04d} failed."
        )

        return False

    return True


def normalize_url(url: str):
    """
    Cho phép truyền cả:

    https://www.tiktok.com/...

    hoặc URL bị copy dạng Markdown:

    [https://...](https://...)
    """

    url = url.strip()

    markdown_match = re.match(
        r"^\[.*?\]\((https?://.+?)\)$",
        url,
    )

    if markdown_match:
        url = markdown_match.group(1)

    return url


def main():
    parser = argparse.ArgumentParser(
        description=(
            "TikTok Short Drama Downloader"
        )
    )

    parser.add_argument(
        "url",
        help="URL của tập đầu tiên",
    )

    parser.add_argument(
        "--episodes",
        type=int,
        default=TOTAL_EPISODES,
        help=(
            "Tổng số tập "
            f"(default: {TOTAL_EPISODES})"
        ),
    )

    parser.add_argument(
        "--cookies",
        default=COOKIE_FILE,
        help=(
            "File cookies.txt "
            f"(default: {COOKIE_FILE})"
        ),
    )

    parser.add_argument(
        "--output",
        default=str(OUTPUT_DIR),
        help=(
            "Thư mục download "
            f"(default: {OUTPUT_DIR})"
        ),
    )

    args = parser.parse_args()

    start_url = normalize_url(
        args.url
    )

    total_episodes = args.episodes
    cookie_file = args.cookies

    output_dir = Path(
        args.output
    )

    if not Path(cookie_file).exists():
        print(
            f"[ERROR] Không tìm thấy "
            f"cookie file: {cookie_file}"
        )
        return

    print("=" * 70)
    print("TikTok Short Drama Downloader")
    print("=" * 70)

    print(
        f"URL      : {start_url}"
    )

    print(
        f"Episodes : {total_episodes}"
    )

    print(
        f"Cookies  : {cookie_file}"
    )

    print(
        f"Output   : {output_dir}"
    )

    print("=" * 70)

    with sync_playwright() as p:

        # GIỮ NGUYÊN cách mở browser
        # vì đây là cấu hình đang chạy được
        browser = p.chromium.launch(
            headless=False,
            channel="chrome",
        )

        context = browser.new_context(
            locale="vi-VN",
        )

        # GIỮ NGUYÊN cách load cookie
        cookies = load_netscape_cookies(
            cookie_file
        )

        print(
            f"[+] Loaded "
            f"{len(cookies)} cookies"
        )

        context.add_cookies(
            cookies
        )

        page = context.new_page()

        print()
        print("[+] Opening EP1...")

        page.goto(
            start_url,
            wait_until="domcontentloaded",
            timeout=120000,
        )

        # Chờ TikTok load/challenge
        page.wait_for_timeout(
            5000
        )

        print(
            f"[+] Opened: {page.url}"
        )

        # Nếu có nút "Xem tất cả xx tập"
        # thì thử mở.
        #
        # Nếu việc click này gây vấn đề trên UI
        # của bạn thì có thể xóa nguyên block này.
        try:
            show_all = page.get_by_text(
                re.compile(
                    r"Xem tất cả .* tập",
                    re.I,
                )
            )

            if show_all.count():
                show_all.first.click()

                page.wait_for_timeout(
                    2000
                )

        except Exception:
            pass

        previous_video_id = None

        for episode in range(
            1,
            total_episodes + 1,
        ):

            print()
            print(
                f"[+] EP {episode}"
            )

            video_url = (
                get_current_video_url(
                    page
                )
            )

            if not video_url:
                print(
                    f"[ERROR] Không lấy "
                    f"được URL EP {episode}"
                )
                break

            current_video_id = (
                get_video_id(
                    video_url
                )
            )

            print(
                f"[+] URL: {video_url}"
            )

            print(
                f"[+] Video ID: "
                f"{current_video_id}"
            )

            # Kiểm tra TikTok có thực sự
            # chuyển sang tập mới hay không
            if (
                previous_video_id
                and current_video_id
                == previous_video_id
            ):
                print(
                    "[!] WARNING: Video ID "
                    "không đổi."
                )

                print(
                    "[!] TikTok có thể chưa "
                    "chuyển sang tập mới."
                )

            download_video(
                url=video_url,
                episode=episode,
                cookie_file=cookie_file,
                output_dir=output_dir,
            )

            previous_video_id = (
                current_video_id
            )

            if (
                episode
                == total_episodes
            ):
                break

            print()
            print(
                f"[+] Scroll EP "
                f"{episode} -> "
                f"EP {episode + 1}"
            )

            go_next_episode(
                page
            )

            time.sleep(1)

        print()
        print(
            "[+] Closing browser..."
        )

        browser.close()

    print()
    print("=" * 70)
    print("DONE")
    print("=" * 70)


if __name__ == "__main__":
    main()