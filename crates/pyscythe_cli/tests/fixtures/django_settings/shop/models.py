class BaseUser:
    @property
    def is_staff(self) -> bool:
        return True


class OrderLine:
    def summary_line(self) -> str:
        return "line"

    def unused_line(self) -> str:
        return "never"
