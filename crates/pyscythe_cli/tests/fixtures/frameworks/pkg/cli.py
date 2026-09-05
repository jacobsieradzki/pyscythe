import click


@click.group()
def cli() -> None:
    pass


@cli.command()
def sync() -> None:
    pass


def main() -> None:
    cli()
