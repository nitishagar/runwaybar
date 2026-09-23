.PHONY: serve check
serve:
	python3 -m http.server 8080 --directory site
check: contrast assets
contrast:
	python3 scripts/check-contrast.py
assets:
	python3 scripts/site-check.py
