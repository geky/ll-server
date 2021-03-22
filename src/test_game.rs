//! test game

use crate::game::*;
use rand::seq::SliceRandom;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::iter;


#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all="snake_case")]
enum TestGameCard {
    Princess,
    Protect,
    Stabby,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all="snake_case")]
enum TestGamePhase {
    BeforeTurn,
    Turn,
    DecidingStabby,
    Ended,
}

#[derive(Debug)]
pub struct TestGame {
    players: Vec<String>,
    current: usize,
    phase: TestGamePhase,

    up_hands: Vec<Vec<TestGameCard>>,
    down_hands: Vec<Vec<TestGameCard>>,
    deck: Vec<TestGameCard>,
    discard: Vec<TestGameCard>,

    log: Vec<String>,
}

// this is sort of a stub, we only allow drawing from the main deck
#[derive(Debug, Serialize, Deserialize, Copy, Clone)]
#[serde(rename_all="snake_case")]
enum TestGameDeck {
    Deck
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag="action", rename_all="snake_case")]
enum TestGameAction {
    Draw {
        user: String,
        deck: TestGameDeck,
    },
    Play {
        user: String,
        card: TestGameCard,
        target: String,
    },
}

impl TestGame {
    pub fn new(mut players: Vec<String>) -> TestGame {
        // shuffle the player order!
        players.shuffle(&mut rand::thread_rng());
        let current = 0;

        // create a deck, sort of arbitrary here
        // princess = 1
        // protect = 1/2 * players
        // stabby = 2 * players
        let mut deck = iter::once(TestGameCard::Princess)
            .chain(iter::repeat(TestGameCard::Protect).take((players.len()+1)/2))
            .chain(iter::repeat(TestGameCard::Stabby).take((players.len()+1)*2))
            .collect::<Vec<_>>();

        // of course
        deck.shuffle(&mut rand::thread_rng());

        // give each player one card at the start
        let down_hands = iter::repeat_with(|| {
                vec![deck.pop().unwrap()]
            })
            .take(players.len())
            .collect::<Vec<_>>();

        // no active cards
        let up_hands = iter::repeat(vec![])
            .take(players.len())
            .collect::<Vec<_>>();

        TestGame {
            players: players,
            current: current,
            phase: TestGamePhase::BeforeTurn,
            down_hands: down_hands,
            up_hands: up_hands,
            deck: deck,
            discard: vec![],
            log: vec![
                format!("Waiting for players..."),
                format!("Shuffling..."),
                format!("Game started"),
            ],
        }
    }

    fn find_player(
        &self,
        user: &str,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        Ok(
            self.players.iter().position(|p| p == user)
                .ok_or_else(|| format!("player {:?} is not playing?", user))?
        )
    }

    fn end_turn(&mut self) {
        // end of game?
        if self.deck.len() == 0 {
            // resolve game!
            // who has the princess?
            let winner = self.down_hands.iter()
                .position(|hand| {
                    hand.iter().any(|c| *c == TestGameCard::Princess)
                });
            match winner {
                Some(winner) => {
                    self.log.push(format!(
                        "{} has the Princess",
                        self.players[winner]
                    ));
                    self.log.push(format!(
                        "{} wins!",
                        self.players[winner]
                    ));
                }
                None => {
                    self.log.push(format!("No one one??"));
                    self.log.push(format!("How did you pull that off?"));
                }
            }

            self.phase = TestGamePhase::Ended;
        } else {
            // move on to next player
            self.current = (self.current+1) % self.players.len();
            self.phase = TestGamePhase::BeforeTurn;

            // clear their up hand
            for card in self.up_hands[self.current].drain(..) {
                self.discard.push(card);
            }
        }
    }
}

impl Game for TestGame {
    fn status(&self) -> String {
        match self.phase {
            TestGamePhase::Ended => format!("ended"),
            _ => format!("in game"),
        }
    }

    fn ended(&self) -> bool {
        match self.phase {
            TestGamePhase::Ended => true,
            _ => false,
        }
    }

    fn state(&self) -> GameState {
        GameState(serde_json::json!({
            "players": self.players,
            "current": self.players[self.current],
            "phase": self.phase,
            "down_hands": self.down_hands.iter()
                .enumerate()
                .map(|(i, hand)| {
                    (&self.players[i], hand)
                })
                .collect::<HashMap<_, _>>(),
            "up_hands": self.up_hands.iter()
                .enumerate()
                .map(|(i, hand)| {
                    (&self.players[i], hand)
                })
                .collect::<HashMap<_, _>>(),
            "decks": [
                {
                    "name": "deck",
                    "card": serde_json::Value::Null,
                    "count": self.deck.len()
                },
                {
                    "name": "discard",
                    "card": self.discard.last(),
                    "count": self.discard.len()
                },
            ],
            "log": self.log,
            "card_imgs": {
                "null": "../../test-card-back.png",
                "princess": "../../test-card-princess.png",
                "protect": "../../test-card-protect.png",
                "stabby": "../../test-card-stabby.png",
            }
        }))
    }

    fn action(
        &mut self,
        action: GameAction,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match (
            serde_json::from_value(action.0)?,
            self.phase
        ) {
            // draw from the deck
            (TestGameAction::Draw{user, deck: _}, TestGamePhase::BeforeTurn) => {
                let user = self.find_player(&user)?;
                if user != self.current {
                    Err(format!("not your turn!"))?;
                }

                // draw card
                let card = self.deck.pop()
                    .ok_or_else(|| format!(
                        "attempted to draw from empty deck?"))?;
                self.down_hands[user].push(card);

                // move on to turn
                self.phase = TestGamePhase::Turn;
                Ok(())
            }
            // let the user know they messed up because this happens a lot
            (TestGameAction::Play{user, ..}, TestGamePhase::BeforeTurn) => {
                let user = self.find_player(&user)?;
                if user != self.current {
                    Err(format!("not your turn!"))?;
                }

                self.log.push(format!(
                    "{} needs to draw..",
                    self.players[user]
                ));

                Ok(())
            }
            // play card, can either be on oneself or on someone else
            (TestGameAction::Play{user, card, target}, TestGamePhase::Turn) => {
                let user = self.find_player(&user)?;
                if user != self.current {
                    Err(format!("not your turn!"))?;
                }

                let target = self.find_player(&target)?;

                // play the card
                match card {
                    TestGameCard::Princess => {
                        // you can't play this one!
                        Err(format!("tried to play princess"))?;
                    }
                    TestGameCard::Protect => {
                        // place card on target's "up" hand (yeah bad name)
                        self.up_hands[target].push(card);
                    }
                    TestGameCard::Stabby => {
                        // can only stab others, but this is currently also
                        // how to discard...
                        if target != user {
                            // are they protected?
                            if
                                self.up_hands[target].iter()
                                    .any(|c| *c == TestGameCard::Protect)
                            {
                                self.log.push(format!(
                                    "{} is protected...",
                                    self.players[target]
                                ));
                                return Ok(());
                            }

                            // need to decide swap
                            let target_card = self.down_hands[target].pop()
                                .ok_or_else(|| format!("target has no cards?"))?;
                            self.down_hands[user].push(target_card);

                            // remove card from hand
                            let i = self.down_hands[user].iter().position(|c| *c == card)
                                .ok_or_else(|| format!("player {:?} doesn't have card {:?}?", user, card))?;
                            self.down_hands[user].remove(i);

                            // update log
                            self.log.push(format!(
                                "{} played {:?} on {}",
                                self.players[user],
                                card,
                                self.players[target],
                            ));

                            self.phase = TestGamePhase::DecidingStabby;
                            return Ok(());
                        }

                        // place on discard
                        self.discard.push(card);
                    }
                }

                // remove card from hand
                let i = self.down_hands[user].iter().position(|c| *c == card)
                    .ok_or_else(|| format!("player {:?} doesn't have card {:?}?", user, card))?;
                self.down_hands[user].remove(i);

                // update log
                self.log.push(format!(
                    "{} played {:?} on {}",
                    self.players[user],
                    card,
                    self.players[target],
                ));

                // move on to next player
                self.end_turn();
                Ok(())
            }
            // decide swap
            (TestGameAction::Play{user, card, target}, TestGamePhase::DecidingStabby) => {
                let user = self.find_player(&user)?;
                if user != self.current {
                    Err(format!("not your turn!"))?;
                }

                let target = self.find_player(&target)?;

                // hm, ok, so we didn't keep track of swap, but we know the
                // one we're swapping with is the only one with no cards...
                // this is sort of a hack, but should work
                if self.down_hands[target].len() > 0 {
                    Err(format!("not swapped player!"))?;
                }

                // remove card from hand
                let i = self.down_hands[user].iter().position(|c| *c == card)
                    .ok_or_else(|| format!("player {:?} doesn't have card {:?}?", user, card))?;
                self.down_hands[user].remove(i);

                // give to target
                self.down_hands[target].push(card);

                // move on to next player
                self.end_turn();
                Ok(())
            }
            _ => {
                Err(format!(
                    "invalid action during phase {:?}",
                    self.phase
                ))?
            }
        }
    }
}
