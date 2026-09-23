import unittest
from unittest.mock import Mock

from tiktok_downloader.browser import TikTokNavigator
from tiktok_downloader.config import TAB_SWITCH_WAIT_MS


class EpisodeNavigationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.page = Mock()
        self.navigator = TikTokNavigator(self.page)

    def test_uses_last_visible_episode_section(self) -> None:
        sections = Mock()
        front_section = Mock()
        sections.last = front_section
        self.page.locator.return_value = sections

        self.assertIs(
            self.navigator._episode_section(),
            front_section,
        )
        self.page.locator.assert_called_once_with(
            'section:visible:has(> h3:text-is("Tập"))'
        )

    def test_does_not_click_the_already_selected_tab(self) -> None:
        tabs = Mock()
        tabs.count.return_value = 4
        self.navigator._episode_tabs = Mock(return_value=tabs)
        self.navigator._detect_selected_tab_index = Mock(return_value=0)

        self.navigator._switch_to_tab(0)

        self.assertEqual(self.navigator._current_tab_index, 0)
        tabs.nth.assert_not_called()
        self.page.wait_for_timeout.assert_not_called()

    def test_clicks_the_next_tab_by_index(self) -> None:
        tabs = Mock()
        tabs.count.return_value = 4
        self.navigator._episode_tabs = Mock(return_value=tabs)
        self.navigator._detect_selected_tab_index = Mock(return_value=0)

        self.navigator._switch_to_tab(1)

        tabs.nth.assert_called_once_with(1)
        tabs.nth.return_value.click.assert_called_once_with()
        self.page.wait_for_timeout.assert_called_once_with(
            TAB_SWITCH_WAIT_MS
        )
        self.assertEqual(self.navigator._current_tab_index, 1)


if __name__ == "__main__":
    unittest.main()
