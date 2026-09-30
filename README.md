# ghdeck

**GitHub i terminalen, à la btop.** Ett statisk binær som viser repoer, Actions, pull requests,
issues, prod/test-deployer, trending og statistikk i ett tastaturstyrt dashbord. Den fungerer mot
både github.com og GitHub Enterprise (Cloud `*.ghe.com` og Server).

```
 ◈ ghdeck  ⌂ acme.ghe.com 1/2  @ kristoffer   ¹dashboard ²repos ³actions ⁴pulls ⁵issues ⁶deploys ⁷explore ⁸inbox   ✉ 3  13:37:00
╭─²repos ───────────── 15 watched ╮╭─³actions ─────────────── ⠹ 4 running ✗ 1 failing ╮╭─⁶deploys ─────────────────────────────╮
│▌✓ checkout-service-app   1   6m ││ ✓ CD – Deploy to env   data-central   main     6m ││ repository        prod          test   │
│ ✓ catalog-service-app    1  14h ││ ⠹ CI/CD                orders-app     feature/ 5m ││ orders-app   ● v0.33.0 2h  ● test 25m  │
│ ✓ ledger-internal-api-v2 1   7m ││ ✗ Deploy               metrics-portal dependa 17m ││ admin-app    ● prod 14h    ● test 5h   │
╰─────────────────────────────────╯╰──────────────────────────────────────────────────╯╰────────────────────────────────────────╯
╭─¹pulse ──────────────────────────╮╭─⁴pulls · review requested ─╮╭─⁵issues · assigned ─╮╭─⁷explore ── trending · top ╮
│2.4k contributions · last year    ││ ⇄ feat: sales-kpi   orders ││ ◎ Migration    1mo  ││  5 acme/platform-standard ●│
│M ■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■  ││ ⇄ fix: pos unblock  ledger ││                     ││  5 acme/data-central      ●│
│W ■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■  ││                            ││                     ││  3 acme/partner-portals   ●│
│ci ok  ━━━━━━━━━━━━━━━━━━━━──  93% ││                            ││                     ││                            │
│api    ━━━━━━━━━━━━━━━━━━━━─ 4.9k  ││                            ││                     ││                            │
│runs 14d ✓ 207 ✗ 15 ▶ 4  ▁▂▅▇█▆█   ││                            ││                     ││                            │
╰──────────────────────────────────╯╰────────────────────────────╯╰─────────────────────╯╰────────────────────────────╯
 NORMAL  dashboard                                                          ⟳ 7s   api 4.9k/5.0k   ␣ leader  ? help
```

## Hva du får

| Visning | Innhold |
| --- | --- |
| **1 dashboard** | btop-rutenett med alle panelene. Tallet i hjørnet er hurtigtasten til fullvisningen. |
| **2 repos** | Fulgte repoer (eller alle med `a`) med CI-status, stjerner og siste push. Detaljpanelet viser språkfordeling, commit-graf for 52 uker (braille), deploy-trinn, siste kjøringer og releaser. |
| **3 actions** | Alle workflow-kjøringer på tvers av repoene. `⏎` viser jobber og steg live. `R` kjører feilede jobber på nytt og `x` avbryter. |
| **4 pulls / 5 issues** | Seksjoner i gh-dash-stil (review requested, mine, involved …) med forhåndsvisning av tekst og etiketter. |
| **6 deploys** | Matrise med repo × miljø (prod/test/…). Hvert trinn slår sammen *Environments*, *workflow-kjøringer* og *releaser*, og den nyeste kilden vinner. |
| **7 explore** | Trending og mest stjernemerkede repoer, med søk som kan konfigureres per host. |
| **8 inbox** | Varsler (notifications). |

Pulse-panelet viser bidragskalender (heatmap), CI-suksessrate, rate-limit-målere og kjøringer
per dag.

## Installasjon

```sh
# fra kildekode (Rust 1.85+)
cargo install --git https://github.com/KristofferRisa/ghdesk

# eller statisk Linux-binær (musl), klar til å kopieres hvor som helst
cargo build --release --target x86_64-unknown-linux-musl
cp target/x86_64-unknown-linux-musl/release/ghdeck ~/.local/bin/
```

Ferdige binærer for Linux (x86_64/arm64, statisk), macOS (arm64/x86_64) og Windows lages av
`release`-workflowen når du pusher en `v*`-tag.

## Autentisering

ghdeck lagrer aldri tokens selv. For hver host prøver den i denne rekkefølgen:

1. `token_env` (navnet på en miljøvariabel)
2. `token_cmd` (f.eks. `pass show github/token`)
3. `gh auth token --hostname <host>`, altså eksisterende `gh`-innlogging
4. `GH_TOKEN`/`GITHUB_TOKEN` (github.com) eller `GH_ENTERPRISE_TOKEN` (enterprise)

Kjør `ghdeck doctor` for å sjekke tokens, endepunkter, rate limit og GraphQL for alle hostene.

## Konfigurasjon

`~/.config/ghdeck/config.toml` (eller `$XDG_CONFIG_HOME/ghdeck/`). Filen opprettes med kommentarer
første gang du starter programmet. Rediger den fra appen med `␣c`, så lastes den inn på nytt når
editoren lukkes. Se [assets/config.example.toml](assets/config.example.toml) for alle valg.

```toml
[general]
refresh_secs = 60        # full oppdatering
fast_refresh_secs = 30   # repoer med kjørende workflows
log_level = "info"       # debug logger hvert API-kall
theme = "tokyonight"     # tokyonight | catppuccin | gruvbox | kanagawa
icons = "nerd"           # nerd | unicode

[[hosts]]
name  = "work"
host  = "acme.ghe.com"                 # *.ghe.com → api.<host>, Server → <host>/api/v3
local_dirs = ["~/code"]                # følg repoene du har klonet lokalt
orgs  = []                             # eller: de sist pushede repoene i en org
auto_repos = 0                         # eller: dine N sist pushede repoer
repos = ["acme/checkout-service-app"]  # alltid øverst
top_query      = "org:acme stars:>=0"
trending_query = "org:acme pushed:>{since:7d}"

[[hosts]]
name = "github"
host = "github.com"

[[deploy.tiers]]
name         = "prod"
environments = ["prod", "production", "prod-*"]  # GitHub Environments (glob)
workflows    = ["*deploy*prod*"]                 # workflow-navn eller -fil (glob)
release      = "latest"                          # nyeste release som ikke er prerelease

[[sections.pulls]]
title  = "Review requested"
filter = "is:open is:pr review-requested:@me"
```

Med `local_dirs` skanner ghdeck mappene ved hver oppdatering og følger alle kloner med en remote
på samme host. Mapper som ikke selv er repoer (f.eks. en mappe med worktrees) sjekkes ett nivå ned.
Skjulte mapper hoppes over. En ny klone dukker opp innen ett minutt.

Det ghdeck skriver selv (fulgte repoer, tema, sist brukte host) lagres i `state.json` i samme
mappe, slik at kommentarene i `config.toml` blir stående.

## Taster

Tastene er inspirert av LazyVim, telescope/snacks.picker og which-key.

| Tast | Handling |
| --- | --- |
| `␣` | leader-meny (which-key), f.eks. `␣␣` repo, `␣f` +find, `␣g` +goto, `␣a` +actions |
| `C-p` / `␣␣` | fuzzy-finn repo (alle du har tilgang til) med forhåndsvisning |
| `:` | kommandopalett |
| `/` | fuzzy-filtrer gjeldende liste (fzf-syntaks: `'eksakt ^start slutt$ !ikke`) |
| `1`–`8` | bytt visning |
| `j/k` `g/G` `C-d/C-u` | naviger |
| `tab` `C-h/j/k/l` | neste panel / fokuser panel i en retning |
| `h/l` `[ ]` | forrige/neste seksjon (pulls, issues, explore) |
| `⏎` | gå inn (repo → kjøringer → jobber) eller åpne |
| `o` / `y` | åpne i nettleser / kopier URL (OSC 52, virker over SSH) |
| `R` / `x` | kjør feilede jobber på nytt / avbryt kjøring (med bekreftelse) |
| `w` | følg / slutt å følge repo |
| `z` | zoom panelet |
| `H` / `t` | neste host / neste tema |
| `r` | oppdater nå |
| `E` / `?` | feillogg / hjelp |
| `S` | API- og ressursbruk: kall per time, 304-andel, CPU, minne, per endepunkt |

## Effektiv bruk av API-et

GitHub gir 5 000 kall i timen per bruker, og kvoten deles med `gh`, Claude Code og alt annet som
bruker samme token. ghdeck prøver å bruke så lite av den som mulig:

- Svar caches og revalideres med `If-Modified-Since` (eller ETag). Uendrede ressurser gir `304`, og
  de belaster ikke rate limit. Med gzip matcher GitHub sjelden sine svake ETag-er, så
  `Last-Modified` foretrekkes. Det er målt på ghe.com.
- Metadata for alle fulgte repoer revalideres hvert minutt, og det er gratis når ingenting er
  endret. Kjøringer hentes bare for repoer som har fått en ny push, er pushet de siste 10 minuttene,
  eller har aktive kjøringer. Resten venter til den langsomme syklusen (10 min).
- Deployer og releaser hentes bare når en deploy-workflow endrer status, en annen kjøring blir
  ferdig, eller etter én time.
- Repoer med kjørende workflows polles hvert `fast_refresh_secs`.
- Når under 40 % av rate limit gjenstår, strekkes intervallene automatisk (vises som `throttled ×N`).
- Detaljer, jobber og commit-statistikk lastes først når markøren har stått stille på et element.
- Skjermen tegnes bare på nytt ved endringer, og ellers én gang i sekundet for klokka. Spinnere gir
  4 bilder i sekundet mens noe kjører. Lange lister bygger bare radene som vises.

Målt i vanlig bruk mot en GitHub Enterprise Cloud-instans med 23 repoer (29.09.2026, én instans, 10-minutterssnitt fra loggen):

| | Før | Etter |
| --- | --- | --- |
| Core-kall per time | ~3 700 (0 % 304) | 600–1 130 (51–61 % gratis 304) |
| Kostnad per vanlig oppdatering | alle repoer, uten cache | 4–8 kall |
| CPU | ~1,3 % | 0,6–0,9 % |
| Minne | 34 MB | 23–25 MB |
| Tid per skjermbilde | – | 1,3–2,1 ms |

Hver instans har sin egen cache og polling. To åpne vinduer koster altså dobbelt.

## Måling og logging

- `S` i appen viser forbruket siden start og for de siste 5 minuttene. Statuslinjen viser
  `↯N/h`, som er ghdecks eget forbruk av core-kvoten per time.
- `ghdeck bench --secs 300 --render` kjører samme planlegger uten terminal og skriver ut kall per
  time, andel 304, CPU, minne og en tabell per endepunkt. `--no-throttle` måler verste tilfelle.
- Loggen ligger i `~/.local/state/ghdeck/ghdeck.log` (`ghdeck log-path`). På `info` får du én linje
  per oppdatering (kall, 304, kostnad, gjenstående kvote og tid), et sammendrag hvert 10. minutt og
  en melding når throttle endres. `GHDECK_LOG=debug` logger hvert enkelt kall. Loggen roteres ved
  5 MB. Tokens logges aldri.

## Arkitektur

```
src/
├── main.rs        CLI (doctor, bench, init, config-path, log-path) og hendelsesløkke
├── app.rs         tilstand, tastekart, which-key-tre, kommandoer, picker-kilder
├── fetch.rs       bakgrunnsjobber → mpsc-meldinger (per host)
├── github/        REST + GraphQL-klient med ETag-cache og rate-limit-sporing
├── deploy.rs      samler Environments, workflows og releaser til deploy-trinn
├── local.rs       finner lokale kloner (local_dirs) og parser git-remotes
├── metrics.rs     tellere for API-kall, 304, CPU, minne og tegning
├── logging.rs     fillogg via tracing
├── fuzzy.rs       nucleo-matcher (fra helix) + utheving av treff
├── config.rs      config.toml, state.json og {since:7d}-maler
└── ui/            ratatui: dashbord, visninger, tabeller, overlegg (picker, which-key, hjelp)
```

Bygget med [ratatui](https://ratatui.rs). Inspirert av
[btop](https://github.com/aristocratos/btop), [gh-dash](https://github.com/dlvhdr/gh-dash),
[LazyVim](https://github.com/LazyVim/LazyVim), [snacks.nvim](https://github.com/folke/snacks.nvim)
og [which-key.nvim](https://github.com/folke/which-key.nvim).
