
use std::time::{Duration, Instant};

use actix::prelude::*;
use actix_files as fs;
use actix_web::{middleware, web, App, HttpRequest, HttpResponse, HttpServer};
use actix_web_actors::ws;
use structopt::StructOpt;
use log::*;
use std::num::ParseIntError;
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::sync::Mutex;
use std::sync::Arc;
use lazy_static::lazy_static;
use rand::Rng;

// game definitions in other files
mod game;
use crate::game::*;
mod test_game;
use crate::test_game::*;

//// game types ////

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
#[serde(rename_all="snake_case")]
enum GameType {
    TestGame,
    OtherTestGame,
}

impl GameType {
    fn names() -> Vec<&'static str> {
        vec!["test_game", "other_test_game"]
    }

    fn create(&self, players: Vec<String>) -> Box<dyn Game> {
        match *self {
            GameType::TestGame      => Box::new(TestGame::new(players)),
            GameType::OtherTestGame => Box::new(TestGame::new(players)),
        }
    }
}


//// random colors ////
const RANDOM_COLORS: &'static [&'static str] = &[
    "#4c72b0", 
    "#dd8452", 
    "#55a868", 
    "#c44e52", 
    "#8172b3", 
    "#937860", 
    "#da8bc3", 
    "#8c8c8c", 
    "#ccb974", 
    "#64b5cd",
];

lazy_static! {
    static ref RANDOM_COLOR_IDX: Mutex<Option<usize>> = Mutex::new(None);
}

fn random_color() -> String {
    let mut idx = RANDOM_COLOR_IDX.lock().unwrap();
    let i = match *idx {
        Some(i) => i,
        None => rand::thread_rng().gen_range(0..RANDOM_COLORS.len()),
    };

    *idx = Some((i + 1) % RANDOM_COLORS.len());
    RANDOM_COLORS[i].to_string()
}


//// game room management ////

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all="snake_case")]
struct GameRoomState {
    #[serde(rename="type")]
    type_: GameType,
    players: Vec<String>,
    status: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag="action", rename_all="snake_case")]
enum GameRoomAction {
    JoinGame {
        name: String
    },
    StartGame,
}

#[derive(Debug)]
struct GameRoom {
    type_: GameType,
    game: Option<Box<dyn Game>>,

    players: Vec<String>,
    player_colors: HashMap<String, String>,

    // may not == players?
    clients: HashMap<Uuid, Recipient<GameState>>,
}

impl GameRoom {
    fn new(type_: GameType) -> Self {
        Self {
            game: None,
            type_: type_,
            players: Vec::new(),
            player_colors: HashMap::new(),
            clients: HashMap::new(),
        }
    }


    /// this is the status of the room for the waiting room
    fn room_state(&self) -> serde_json::Value {
        serde_json::json!({
            "type": self.type_,
            "players": self.players,
            "status": "waiting on players...",
        })
    }

    fn broadcast_state(&self) {
        // broadcast update
        let state = self.state();
        for (_, client) in self.clients.iter() {
            client.do_send(state.clone()).warn_err().ok();
        }
    }

    /// game state
    fn state(&self) -> GameState {
        // inject player info
        GameState(serde_json::json!({
            "game": self.game.as_ref().map(|game| {
                game.state()
            }),
            "players": self.players,
            "player_colors": self.player_colors,
        }))
    }

    /// game actions
    fn action(
        &mut self,
        action: GameAction
    ) -> Result<(), Box<dyn std::error::Error>> {
        // intercept non-game specific actions
        let res = match (
            serde_json::from_value::<GameRoomAction>(action.0.clone()),
            &mut self.game
        ) {
            (Ok(action), _) => {
                match action {
                    GameRoomAction::JoinGame{name} => {
                        // already a player?
                        if !self.player_colors.contains_key(&name) {
                            self.players.push(name.to_string());
                            // get a new color
                            self.player_colors.insert(
                                name.to_string(),
                                random_color()
                            );
                        }
                        Ok(())
                    }
                    GameRoomAction::StartGame => {
                        // people are definitely going to click this a bunch,
                        // so do nothing if game is already in play
                        if 
                            match &self.game {
                                None => true,
                                Some(game) => game.ended(),
                            }
                        {
                            // start the game!
                            self.game = Some(
                                self.type_.create(self.players.clone())
                            );
                        }
                        Ok(())
                    }
                }
            }
            (_, Some(game)) => {
                // ignore that, continue to game action
                game.action(action)
            }
            (Err(err), _) => {
                Err(err)?
            }
        };

        if res.is_ok() {
            self.broadcast_state();
        }

        res
    }
}

#[derive(Debug)]
struct GameRoomClient {
    addr: String,
    uuid: Uuid,
    heartbeat: Duration,
    heartbeat_last: Instant,

    room_name: String,
    room: Arc<Mutex<GameRoom>>,
}

impl GameRoomClient {
    fn new(
        addr: &str,
        heartbeat: Duration,
        room_name: &str,
        room: Arc<Mutex<GameRoom>>,
    ) -> Self {
        Self {
            addr: addr.to_string(),
            uuid: Uuid::new_v4(),
            heartbeat: heartbeat,
            heartbeat_last: Instant::now(),
            room_name: room_name.to_string(),
            room: room,
        }
    }

    async fn get(
        request: HttpRequest,
        stream: web::Payload,
        opt: web::Data<Opt>,
        room: web::Path<(String, String)>,
    ) -> actix_web::Result<HttpResponse> {
        // find game room from global waiting room
        let room_name = &room.into_inner().0;
        let room = WAITING_ROOM.lock().unwrap().rooms.get(room_name)
            .ok_or_else(|| { warn!("can't find room {}", room_name); () })?
            .clone();

        ws::start(
            GameRoomClient::new(
                request.connection_info()
                    .remote_addr()
                    .ok_or_else(|| { warn!("no remote addr?"); () })?,
                opt.heartbeat,
                room_name,
                room,
            ),
            &request,
            stream
        )
    }
}

impl Actor for GameRoomClient {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        info!("client {} connected", self.addr);

        // keep track of clients for broadcasts
        self.room.lock().unwrap().clients.insert(
            self.uuid, 
            ctx.address().recipient(),
        );
            
        // heartbeat to catch disconnects
        ctx.run_interval(self.heartbeat, |act, ctx| {
            if Instant::now()
                    .duration_since(act.heartbeat_last) > 2*act.heartbeat {
                ctx.stop();
                return
            }

            ctx.ping(b"");
        });

        // update with room info
        ctx.address().do_send(self.room.lock().unwrap().state());
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        let is_empty = {
            // keep track of waiter for broadcasts
            let mut room = self.room.lock().unwrap();
            room.clients.remove(&self.uuid);
            room.clients.is_empty()
        };

        // clean up room if all clients have left
        if is_empty {
            match
                WAITING_ROOM.lock().unwrap()
                    .action(WaitingRoomAction::DestroyRoom {
                        room_name: self.room_name.clone()
                    })
            {
                Ok(()) => {},
                Err(err) => {
                    warn!("{}", err);
                }
            }
        }

        info!("client {} disconnected", self.addr);
    }
}

impl StreamHandler<
    Result<ws::Message, ws::ProtocolError>
> for GameRoomClient {
    fn handle(
        &mut self,
        msg: Result<ws::Message, ws::ProtocolError>,
        ctx: &mut Self::Context,
    ) {
        // process websocket messages
        match msg {
            Ok(ws::Message::Ping(msg)) => {
                self.heartbeat_last = Instant::now();
                ctx.pong(&msg);
            }
            Ok(ws::Message::Pong(_)) => {
                self.heartbeat_last = Instant::now();
            }
            Ok(ws::Message::Close(reason)) => {
                ctx.close(reason);
                ctx.stop();
            }
            Ok(ws::Message::Text(ref text)) => {
                match
                    serde_json::from_str::<GameAction>(&text)
                        .map_err(|err| -> Box<dyn std::error::Error> {
                            Box::new(err)
                        })
                        .and_then(|action| {
                            self.room.lock().unwrap().action(action)
                        })
                        .and_then(|_| {
                            // also broadcast game room updates to waiting room,
                            // this is sort of a hack
                            WAITING_ROOM.lock().unwrap().broadcast_state();
                            Ok(())
                        })
                {
                    Ok(()) => (),
                    Err(err) => {
                        warn!("bad message {:?}", &msg);
                        warn!("{}", err);
                    }
                };
            }
            _ => {
                warn!("bad message {:?}", msg);
            }
        }
    }
}

impl Handler<GameState> for GameRoomClient {
    type Result = ();

    fn handle(
        &mut self,
        msg: GameState,
        ctx: &mut Self::Context
    ) -> Self::Result {
        // broadcast updates to all connected clients
        let json = match serde_json::to_string(&msg) {
            Ok(json) => json,
            Err(err) => {
                warn!("{}", err);
                return
            }
        };

        ctx.text(json);
    }
}


//// waiting room management ////

/// landing page is a simple waiting room
#[derive(Debug)]
struct WaitingRoom {
    rooms: HashMap<String, Arc<Mutex<GameRoom>>>,
    waiters: HashMap<Uuid, Recipient<WaitingRoomState>>,
}

impl WaitingRoom {
    fn new() -> Self {
        Self {
            rooms: HashMap::new(),
            waiters: HashMap::new(),
        }
    }

    fn state(&self) -> WaitingRoomState {
        WaitingRoomState(serde_json::json!({
            "rooms": self.rooms.iter()
                .map(|(name, room)| (
                    name.to_string(),
                    room.lock().unwrap().room_state()
                ))
                .collect::<HashMap<_, _>>()
        }))
    }

    fn broadcast_state(&self) {
        // broadcast update
        let state = self.state();
        for (_, client) in self.waiters.iter() {
            client.do_send(state.clone()).warn_err().ok();
        }
    }

    fn create_room(
        &mut self,
        room_name: &str,
        room_type: GameType
    ) -> Result<(), Box<dyn std::error::Error>> {
        // keep track of rooms
        if room_name.len() == 0 {
            Err(format!("can't create room without name"))?;
        }

        if self.rooms.contains_key(room_name) {
            Err(format!("room already exists {:?}", room_name))?;
        }

        self.rooms.insert(
            room_name.to_string(),
            Arc::new(Mutex::new(GameRoom::new(room_type)))
        );

        Ok(())
    }

    fn destroy_room(
        &mut self,
        room_name: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match self.rooms.remove(room_name) {
            Some(_) => (),
            None => Err(format!("room does not exist? {:?}", room_name))?,
        }

        Ok(())
    }

    fn action(
        &mut self,
        action: WaitingRoomAction
    ) -> Result<(), Box<dyn std::error::Error>> {
        let res = match action {
            WaitingRoomAction::CreateRoom{room_name, room_type} => {
                info!("creating room {:?} type {:?}", room_name, room_type);
                self.create_room(&room_name, room_type)
            }
            WaitingRoomAction::DestroyRoom{room_name} => {
                info!("destroying room {:?}", room_name);
                self.destroy_room(&room_name)
            }
        };

        if res.is_ok() {
            self.broadcast_state();
        }

        res
    }
}

lazy_static! {
    /// global waiting room state
    static ref WAITING_ROOM: Mutex<WaitingRoom> = {
        Mutex::new(WaitingRoom::new())
    };
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag="action", rename_all="snake_case")]
enum WaitingRoomAction {
    CreateRoom {
        room_name: String,
        room_type: GameType,
    },
    DestroyRoom {
        room_name: String,
    },
}

#[derive(Debug, Message, Serialize, Deserialize, Clone)]
#[serde(rename_all="snake_case")]
#[rtype(result="()")]
struct WaitingRoomState(serde_json::Value);

#[derive(Debug)]
struct WaitingRoomClient {
    addr: String,
    uuid: Uuid,
    heartbeat: Duration,
    heartbeat_last: Instant,
}

impl WaitingRoomClient {
    fn new(addr: &str, heartbeat: Duration) -> Self {
        Self {
            addr: addr.to_string(),
            uuid: Uuid::new_v4(),
            heartbeat: heartbeat,
            heartbeat_last: Instant::now(),
        }
    }

    async fn get(
        request: HttpRequest,
        stream: web::Payload,
        opt: web::Data<Opt>,
    ) -> actix_web::Result<HttpResponse> {
        ws::start(
            WaitingRoomClient::new(
                request.connection_info()
                    .remote_addr()
                    .ok_or_else(|| { warn!("no remote addr?"); () })?,
                opt.heartbeat,
            ),
            &request,
            stream
        )
    }
}

impl Actor for WaitingRoomClient {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        info!("client {} connected", self.addr);

        // keep track of waiter for broadcasts
        WAITING_ROOM.lock().unwrap().waiters.insert(
            self.uuid,
            ctx.address().recipient(),
        );
            
        // heartbeat to catch disconnects
        ctx.run_interval(self.heartbeat, |act, ctx| {
            if Instant::now()
                    .duration_since(act.heartbeat_last) > 2*act.heartbeat {
                ctx.stop();
                return
            }

            ctx.ping(b"");
        });

        // update with room info
        ctx.address().do_send(WAITING_ROOM.lock().unwrap().state());
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        // keep track of waiter for broadcasts
        WAITING_ROOM.lock().unwrap().waiters.remove(&self.uuid);
        info!("client {} disconnected", self.addr);
    }
}

impl StreamHandler<
    Result<ws::Message, ws::ProtocolError>
> for WaitingRoomClient {
    fn handle(
        &mut self,
        msg: Result<ws::Message, ws::ProtocolError>,
        ctx: &mut Self::Context,
    ) {
        // process websocket messages
        match msg {
            Ok(ws::Message::Ping(msg)) => {
                self.heartbeat_last = Instant::now();
                ctx.pong(&msg);
            }
            Ok(ws::Message::Pong(_)) => {
                self.heartbeat_last = Instant::now();
            }
            Ok(ws::Message::Close(reason)) => {
                ctx.close(reason);
                ctx.stop();
            }
            Ok(ws::Message::Text(ref text)) => {
                match
                    serde_json::from_str(&text)
                        .map_err(|err| -> Box<dyn std::error::Error> {
                            Box::new(err)
                        })
                        .and_then(|action| {
                            WAITING_ROOM.lock().unwrap().action(action)
                        })
                {
                    Ok(()) => (),
                    Err(err) => {
                        warn!("bad message {:?}", &msg);
                        warn!("{}", err);
                    }
                };
            }
            _ => {
                warn!("bad message {:?}", msg);
            }
        }
    }
}

impl Handler<WaitingRoomState> for WaitingRoomClient {
    type Result = ();

    fn handle(
        &mut self,
        msg: WaitingRoomState,
        ctx: &mut Self::Context
    ) -> Self::Result {
        // broadcast updates to all connected clients
        let json = match serde_json::to_string(&msg) {
            Ok(json) => json,
            Err(err) => {
                warn!("{}", err);
                return
            }
        };

        ctx.text(json);
    }
}


//// entry point below ////
fn parse_duration(s: &str) -> Result<Duration, ParseIntError> {
    Ok(Duration::from_secs(s.parse::<u64>()?))
}

#[derive(StructOpt, Debug, Clone)]
#[structopt(rename_all="kebab")]
struct Opt {
    /// Addr to bind server to
    #[structopt(default_value="0.0.0.0:1234")]
    addr: String,

    /// Heartbeat duration in seconds 
    #[structopt(
        short, long, default_value="5",
        parse(try_from_str=parse_duration)
    )]
    heartbeat: Duration,
}

trait ErrorEx {
    fn warn_err(self) -> Self;
}

impl<T, E: std::fmt::Display> ErrorEx for Result<T, E> {
    fn warn_err(self) -> Self {
        self.map_err(|e| {
            warn!("{}", e);
            e
        })
    }
}

#[actix_web::get("/")]
async fn waiting_room() -> actix_web::Result<HttpResponse> {
    let body = std::fs::read("static/waiting-room.html").warn_err()?;
    let body = String::from_utf8_lossy(&body)
        .replace(
            "ROOM_TYPES",
            &serde_json::to_string(&GameType::names()).warn_err()?
        )
        // landing page gets a random color, because why not
        .replace(
            "RANDOM_COLOR",
            &serde_json::to_string(&random_color())?
        );
    Ok(HttpResponse::Ok().body(body))
}

#[actix_web::get("/room/{room}/{user}")]
async fn game_room(
    room: web::Path<(String, String)>
) -> actix_web::Result<HttpResponse> {
    println!("HEY");
    let body = std::fs::read("static/game-room.html").warn_err()?;
    let body = String::from_utf8_lossy(&body)
        .replace("ROOM", &serde_json::to_string(&room.0).warn_err()?)
        .replace("USER", &serde_json::to_string(&room.1).warn_err()?);
    Ok(HttpResponse::Ok().body(body))
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // args
    let opt = Opt::from_args();
    let addr = opt.addr.clone();

    // setup logging
    std::env::set_var("RUST_LOG", "info");
    env_logger::init();

    info!("launching server on {}", addr);

    // launch server
    HttpServer::new(move || {
        App::new()
            // enable logger
            .wrap(middleware::Logger::default())
            // pass options
            .data(opt.clone())
            // dynamic files
            .service(waiting_room)
            .service(game_room)
            // websocket routes
            .service(
                web::resource("/ws")
                    .route(web::get().to(WaitingRoomClient::get))
            )
            .service(
                web::resource("/room/{room}/{user}/ws")
                    .route(web::get().to(GameRoomClient::get))
            )
            // static files
            .service(fs::Files::new("/", "static/"))
    })
    .bind(addr)?
    .run()
    .await
}
