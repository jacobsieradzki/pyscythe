from pkg.widget import Widget


class TestWidget:
    def setup_method(self) -> None:
        self.widget = Widget(1)

    def test_render(self) -> None:
        assert self.widget.render() == "x"

    def helper_nobody_calls(self) -> None:
        pass
