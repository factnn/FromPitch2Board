# Task interface

One decision point, one JSON observation in, one JSON action out.

A decision point arises on a matchday, when a new offer arrives (**Manager** only), or on a day inside an open transfer window (**Recruiter** and **Manager** only). When triggers coincide the runner ranks them matchday first, offer second, market day third, so one decision always takes one action.

## Observation

| Field | Contents |
|---|---|
| `step`, `date` | decision index and simulated date |
| `team_name`, `formation` | managed club and current formation |
| `league_position`, `points` | league standing |
| `budget`, `transfer_window_open` | transfer budget and window state |
| `is_matchday`, `next_fixture` | whether a fixture is due today, and its description |
| `squad` | our players: `id`, `name`, `position`, `group_position`, `ovr`, `age`, `condition`, `fitness`, `morale`, `injured`, `transfer_listed`, `wage`, `market_value` |
| `market` | listed players: `player_id`, `player_name`, `position`, `age`, `market_value`, `team`, `reported_ovr`, `potential`, `scouting` |
| `offers` | pending incoming offers: `offer_id`, `player_id`, `player_name`, `from_team`, `fee`, `round`, `suggested_counter` |
| `scout_reports`, `scouting_in_progress` | completed and running scouting assignments |
| `done` | episode finished |

Market ratings are hidden until the player is scouted: `reported_ovr` and `potential` are present only for players with a scout report, and the reported rating is fuzzed.

The runner returns the outcome of the previous action in `last_action_result` after each action.

## Actions

| Action | Parameters | Coach | Recruiter | Manager |
|---|---|:---:|:---:|:---:|
| `Continue` | — (skipped decision) | • | • | • |
| `SetLineup` | `player_ids` | • | • | • |
| `SetTactics` | `play_style` | • | • | • |
| `SetMatchPlan` | `player_ids`, `play_style` | • | • | • |
| `Substitute` | `player_out_id`, `player_in_id` | • | • | • |
| `MatchTactics` | `play_style`, `formation` | • | • | • |
| `Scout` | `player_id` | | • | • |
| `MakeBid` | `player_id`, `fee` | | • | • |
| `AcceptOffer` | `player_id`, `offer_id` | | | • |
| `RejectOffer` | `player_id`, `offer_id` | | | • |
| `CounterOffer` | `player_id`, `offer_id`, `fee` | | | • |
| `ListPlayer` | `player_id` | | | • |

The responsibility scope is fixed for the whole episode and enforced by the runner: an out-of-scope action is rejected with an explanatory message and consumes the decision point.

Within a match the runner stops the clock only for `Substitute`, `MatchTactics` and `Continue`, the three in-match actions that every scope holds.

## Example exchange

```
observation  {"step": 0, "date": "2026-07-01",
              "is_matchday": true, "points": 0,
              "league_position": 19, "budget": 5000000,
              "transfer_window_open": true, "offers": [],
              "next_fixture":
                "2026-07-01 Club_06 vs Club_32 (A)",
              "squad": [{"id": "bc351785-...",
                         "name": "Player_683",
                         "position": "Goalkeeper", "ovr": 73,
                         "age": 34, "condition": 89,
                         "fitness": 75, "morale": 48,
                         "injured": false,
                         "transfer_listed": false,
                         "wage": 5329,
                         "market_value": 1065800}, ...]}
action       {"action": "SetMatchPlan",
              "params": {"player_ids":
                ["bc351785-...", ...11 ids...],
                "play_style": "Defensive"}}
environment  "Set lineup (11 of 11 provided ids are in
              your squad) and Defensive tactics for the
              next match."
```

## Objective

Each scenario ships a qualitative goal in the prompt (e.g. *avoid relegation* in the crisis scenario, *finish in the top half while keeping net transfer spend small* in the moneyball scenario), and episodes are scored by league points. The agent never sees the calibration statistics or the normalisation that builds the reported Z-score.

## Runner rules

The action set contains no contract renewal and the observation does not expose contract end dates: a player whose contract expires leaves the squad automatically, and the agent sees only that the player is gone. Squad maintenance therefore runs through bids, sales and transfer-listing, never through retention.

Lineups are slot-aligned: the engine keeps the requested starters and fills every empty slot with the best available player for that slot, and if fewer than eight of the requested players are still available it discards the request and selects an automatic best-fit eleven. No minimum-squad rule applies and no error is raised, so a squad that cannot field eleven players degrades silently.

## Greedy reference

The greedy reference is a scripted policy that sees exactly the same observation as the LLM agents and consumes one action per decision point. At each decision point it enumerates the actions its scope permits, scores the resulting selections with a hand-written utility

```
U = ovr + role_fit - 0.5 * (100 - condition) - 0.2 * (100 - fitness)
```

where `role_fit` subtracts 8 for fielding a player out of position and injured players are excluded, and takes the highest-scoring action.

In the **Manager** scope it also bids for players who would raise the squad average by at least 4, scouts before bidding, prices bids for value, accepts offers at 1.1x valuation for surplus players and 1.8x for starters, and lists non-starters aged 27 or over.
