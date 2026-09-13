from shop.models import OrderLine


class OrderTests:
    def test_summary(self) -> None:
        assert OrderLine().summary_line()
