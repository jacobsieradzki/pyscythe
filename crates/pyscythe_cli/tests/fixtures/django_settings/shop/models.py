class BaseUser:
    @property
    def is_staff(self) -> bool:
        return True
