from setuptools import setup

setup(
    name="legacy",
    entry_points={
        "console_scripts": ["legacy = pkg.cli:main"],
    },
)
