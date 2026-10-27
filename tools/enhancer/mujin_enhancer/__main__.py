"""`python -m mujin_enhancer serve`: starts the worker.

On the first start it creates the shared secret (shown in this window) and,
unless `--offline`, downloads the Apollo weights once into the cache.
"""

from __future__ import annotations

import argparse
import os
import sys
from pathlib import Path

from . import __version__
from .config import Config, default_home, load_or_create_secret


def parse(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(prog="mujin-enhancer", description=__doc__)
    sub = parser.add_subparsers(dest="command")
    serve = sub.add_parser("serve", help="run the worker")
    serve.add_argument("--host", default=os.environ.get("MUJIN_ENHANCER_HOST", "0.0.0.0"))  # noqa: S104
    serve.add_argument(
        "--port", type=int, default=int(os.environ.get("MUJIN_ENHANCER_PORT", "8765"))
    )
    serve.add_argument(
        "--model",
        default=os.environ.get("MUJIN_ENHANCER_MODEL", "apollo"),
        help="apollo (default); passthrough is a do-nothing model for testing",
    )
    serve.add_argument(
        "--device",
        default=os.environ.get("MUJIN_ENHANCER_DEVICE", "auto"),
        choices=["auto", "cuda", "cpu"],
    )
    serve.add_argument("--home", type=Path, default=default_home(), help="secret, models and jobs")
    serve.add_argument(
        "--offline",
        action="store_true",
        default=os.environ.get("MUJIN_ENHANCER_OFFLINE") == "1",
        help="never download anything; the weights must already be in the cache",
    )
    serve.add_argument(
        "--shared-root",
        action="append",
        type=Path,
        default=[
            Path(p)
            for p in os.environ.get("MUJIN_ENHANCER_SHARED_ROOTS", "").split(os.pathsep)
            if p
        ],
        help="a music folder this worker may read by path (repeatable)",
    )
    serve.add_argument("--no-benchmark", action="store_true", help="skip the speed test")
    sub.add_parser("secret", help="print the shared secret")
    sub.add_parser("version", help="print the version")
    args = parser.parse_args(argv)
    if args.command is None:
        args = parser.parse_args(["serve", *argv])
    return args


def main(argv: list[str] | None = None) -> None:
    args = parse(sys.argv[1:] if argv is None else argv)
    if args.command == "version":
        print(__version__)
        return
    if args.command == "secret":
        secret, _ = load_or_create_secret(Config(home=default_home()))
        print(secret)
        return
    config = Config(
        home=args.home,
        host=args.host,
        port=args.port,
        model=args.model,
        device=args.device,
        offline=args.offline,
        shared_roots=args.shared_root,
    )
    secret, created = load_or_create_secret(config)
    print(f"Mujin Music enhancer {__version__}")
    print(f"Data folder: {config.home}")
    print("")
    print("Shared secret (paste it into Mujin Music: Settings > Better versions > AI enhancer):")
    print(f"    {secret}")
    if created:
        print("    (new: created on this first start and saved in the data folder)")
    print("")

    from .models import load_model

    model = load_model(config.model, config.device, config.cache_dir, config.offline)
    realtime_factor = None
    if not args.no_benchmark:
        from .models.apollo import measure_realtime_factor

        print("Measuring speed...")
        realtime_factor = measure_realtime_factor(model)
        verdict = "fast enough for real time" if realtime_factor < 0.5 else "too slow for real time"
        print(f"{model.device_name}: {realtime_factor:.2f} s per second of audio ({verdict}).")
    if model.device == "cpu" and model.name != "passthrough":
        print("Warning: running on the CPU. Enhancing works but is slow.")

    import uvicorn

    from .server import Worker, create_app

    worker = Worker(config, model, secret, realtime_factor)
    print(f"Listening on http://{config.host}:{config.port}")
    uvicorn.run(create_app(worker), host=config.host, port=config.port, log_level="warning")


if __name__ == "__main__":
    main()
