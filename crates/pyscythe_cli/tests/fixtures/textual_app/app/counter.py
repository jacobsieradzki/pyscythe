from textual.widget import Widget


class Counter(Widget):
    def _on_mount(self):
        self.total = 0

    def on_click(self):
        self.total += 1

    def helper(self):
        """Not a handler, and nothing calls it."""
        return self.total


class Plain:
    def _on_mount(self):
        """Not a message pump, so nothing dispatches to this."""
        return None
