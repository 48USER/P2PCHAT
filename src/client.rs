use futures::stream::StreamExt;
use libp2p::Swarm;
use libp2p::{
    gossipsub,
    mdns,
    noise,
    swarm::NetworkBehaviour,
    swarm::SwarmEvent,
    tcp,
    yamux,
    PeerId,
    StreamProtocol,
};
use libp2p_request_response::{ ProtocolSupport, ResponseChannel };
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::collections::HashSet;
use std::error::Error;
use std::hash::{ Hash, Hasher };
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;
use tokio::{ io as aio, io::AsyncBufReadExt };

use crate::storage::Storage;
use crate::protocols::{ FileOrMenuRequest, FileOrMenuResponse };
use crate::utils::{
    errprintln,
    sysprintln,
    read_file_as_bytes,
    save_bytes_to_file,
    ensure_directory_exists,
    list_files_in_directory,
};

#[derive(NetworkBehaviour)]
#[behaviour(out_event = "MyBehaviourEvent")]
pub struct MyBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub mdns: mdns::tokio::Behaviour,
    pub request_response: libp2p_request_response::cbor::Behaviour<
        FileOrMenuRequest,
        FileOrMenuResponse
    >,
}

#[derive(Debug)]
pub enum MyBehaviourEvent {
    Gossipsub(gossipsub::Event),
    Mdns(mdns::Event),
    RequestResponse(libp2p_request_response::Event<FileOrMenuRequest, FileOrMenuResponse>),
}

impl From<gossipsub::Event> for MyBehaviourEvent {
    fn from(event: gossipsub::Event) -> Self {
        MyBehaviourEvent::Gossipsub(event)
    }
}

impl From<mdns::Event> for MyBehaviourEvent {
    fn from(event: mdns::Event) -> Self {
        MyBehaviourEvent::Mdns(event)
    }
}

impl From<libp2p_request_response::Event<FileOrMenuRequest, FileOrMenuResponse>>
for MyBehaviourEvent {
    fn from(event: libp2p_request_response::Event<FileOrMenuRequest, FileOrMenuResponse>) -> Self {
        MyBehaviourEvent::RequestResponse(event)
    }
}

pub struct Client {
    pub storage: Storage,
    pub topic_map: HashMap<String, gossipsub::IdentTopic>,
    pub swarm: Swarm<MyBehaviour>,
    pub accomplices: HashSet<String>,
    pub accomplices_nicknames: HashMap<String, String>,
    pub pub_dir_path: String,
    pub requests_mailbox: HashMap<
        u64,
        ((String, ResponseChannel<FileOrMenuResponse>), FileOrMenuRequest)
    >,
    pub pending_requests: Vec<(String, FileOrMenuRequest)>,
}

impl Client {
    pub fn new(
        kr_file_path: &str,
        pub_dir_path: &str
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let mut swarm = libp2p::SwarmBuilder
            ::with_new_identity()
            .with_tokio()
            .with_tcp(tcp::Config::default(), noise::Config::new, yamux::Config::default)
            .map_err(|e| Box::<dyn Error + Send + Sync>::from(e.to_string()))?
            .with_quic()
            .with_behaviour(
                |key| -> Result<MyBehaviour, Box<dyn Error + Send + Sync>> {
                    let message_id_fn = |message: &gossipsub::Message| {
                        let mut s = DefaultHasher::new();
                        message.data.hash(&mut s);
                        gossipsub::MessageId::from(s.finish().to_string())
                    };
                    let gossipsub_config = gossipsub::ConfigBuilder
                        ::default()
                        .heartbeat_interval(Duration::from_secs(10))
                        .validation_mode(gossipsub::ValidationMode::Strict)
                        .message_id_fn(message_id_fn)
                        .build()
                        .map_err(|msg| Box::<dyn Error + Send + Sync>::from(msg))?;
                    let gossipsub = gossipsub::Behaviour
                        ::new(gossipsub::MessageAuthenticity::Signed(key.clone()), gossipsub_config)
                        .map_err(|e| Box::<dyn Error + Send + Sync>::from(e.to_string()))?;
                    let mdns = mdns::tokio::Behaviour
                        ::new(mdns::Config::default(), key.public().to_peer_id())
                        .map_err(|e| Box::<dyn Error + Send + Sync>::from(e.to_string()))?;
                    let request_response = libp2p_request_response::cbor::Behaviour::new(
                        [(StreamProtocol::new("/file-exchange/1"), ProtocolSupport::Full)],
                        libp2p_request_response::Config
                            ::default()
                            .with_request_timeout(Duration::from_secs(3600))
                    );
                    Ok(MyBehaviour {
                        gossipsub,
                        mdns,
                        request_response,
                    })
                }
            )?
            .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
            .build();

        let topic = gossipsub::IdentTopic::new("global");
        swarm
            .behaviour_mut()
            .gossipsub.subscribe(&topic)
            .map_err(|e| Box::<dyn Error + Send + Sync>::from(e.to_string()))?;
        let mut storage = Storage::new(kr_file_path).map_err(|e|
            Box::<dyn Error + Send + Sync>::from(e.to_string())
        )?;
        let mut topic_map: HashMap<String, gossipsub::IdentTopic> = HashMap::new();
        topic_map.insert("global".to_string(), topic.clone());
        let mut errored_keys = Vec::new();
        for (room, cur_key) in &storage.keys {
            let cur_topic = gossipsub::IdentTopic::new(cur_key.clone());
            if let Err(e) = swarm.behaviour_mut().gossipsub.subscribe(&cur_topic) {
                errprintln(&format!("Error unlocking room: {:?}", e));
                errored_keys.push(cur_key.clone());
            } else {
                topic_map.insert(room.to_string(), cur_topic.clone());
            }
        }
        for cur_key in errored_keys {
            storage.remove_key(&cur_key);
        }
        storage.add_key("global".to_string(), "global".to_string());
        if let Err(e) = ensure_directory_exists(pub_dir_path) {
            errprintln(&format!("Error: {}", e));
        }
        if let Err(e) = ensure_directory_exists("./drain") {
            errprintln(&format!("Error: {}", e));
        }
        Ok(Client {
            storage,
            topic_map,
            swarm,
            accomplices: HashSet::new(),
            accomplices_nicknames: HashMap::new(),
            pub_dir_path: pub_dir_path.to_string(),
            requests_mailbox: HashMap::new(),
            pending_requests: Vec::new(),
        })
    }

    pub fn unlock_room(
        &mut self,
        room_name: &str,
        key: &str
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let topic = gossipsub::IdentTopic::new(key);
        self.swarm
            .behaviour_mut()
            .gossipsub.subscribe(&topic)
            .map_err(|e| Box::<dyn Error + Send + Sync>::from(e.to_string()))?;
        self.storage.add_key(room_name.to_string(), key.to_string());
        self.topic_map.insert(room_name.to_string(), topic);
        Ok(())
    }

    pub async fn request_file(&mut self, peer_id: &PeerId, file_name: &str, message: &str) {
        let request = FileOrMenuRequest {
            file_name: file_name.to_string(),
            message: message.to_string(),
            is_menu: false,
        };
        self.swarm.behaviour_mut().request_response.send_request(peer_id, request.clone());
        self.pending_requests.push((peer_id.to_string(), request));
    }

    pub async fn request_menu(&mut self, peer_id: &PeerId) {
        let request = FileOrMenuRequest {
            file_name: "".to_string(),
            message: "".to_string(),
            is_menu: true,
        };
        self.swarm.behaviour_mut().request_response.send_request(peer_id, request);
    }

    pub fn get_menu(&mut self) -> Vec<String> {
        list_files_in_directory(&self.pub_dir_path).unwrap()
    }

    pub fn close(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.storage.save_to_file().map_err(|e| Box::<dyn Error + Send + Sync>::from(e.to_string()))
    }

    pub fn get_accomplice_by_nickname(&self, nickname: &str) -> Option<&String> {
        for (acc, cur_nickname) in &self.accomplices_nicknames {
            if nickname == cur_nickname {
                return Some(acc);
            }
        }
        None
    }

    fn list_accomplices(&self) {
        sysprintln("Accomplices:");
        for id in &self.accomplices {
            println!(
                "\t{} {}",
                id,
                self.accomplices_nicknames.get(id).unwrap_or(&"-/-".to_string())
            );
        }
    }

    fn list_rooms(&self) {
        sysprintln("Rooms:");
        for room in self.topic_map.keys() {
            println!("\t{}", room);
        }
    }

    fn list_outcoming_requests(&self) {
        sysprintln("Outcoming Requests:");
        for (dest, request) in &self.pending_requests {
            println!("\t |Request To: {}", self.accomplices_nicknames.get(dest).unwrap_or(dest));
            println!("\t |Requested File: {}", request.file_name);
            println!("\t |Request  Message: {}", request.message);
            println!("\t ---");
        }
    }

    fn list_incoming_requests(&self) {
        sysprintln("Incoming Requests:");
        for (id, (requester, request)) in &self.requests_mailbox {
            println!("\t |Request Id: {}", id);
            println!(
                "\t |Request From: {}",
                self.accomplices_nicknames.get(&requester.0).unwrap_or(&requester.0)
            );
            println!("\t |Requested File: {}", request.file_name);
            println!("\t |Request  Message: {}", request.message);
            println!("\t ---");
        }
    }

    pub async fn run(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let mut stdin = aio::BufReader::new(aio::stdin()).lines();
        self.swarm.listen_on("/ip4/0.0.0.0/udp/0/quic-v1".parse()?)?;
        self.swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;
        loop {
            tokio::select! {
                Ok(Some(line)) = stdin.next_line() => {
                    let trimmed = line.trim();
                    let mut parts = trimmed.split_whitespace();
                    match parts.next() {
                        Some("sub") => {
                            if let (Some(room), Some(key)) = (parts.next(), parts.next()) {
                                if let Err(e) = self.unlock_room(room, key) {
                                    errprintln(&format!("Error unlocking room: {:?}", e));
                                }
                            } else {
                                errprintln("Usage: sub <room> <key>");
                            }
                        }
                        Some("unsub") => {
                            if let Some(room) = parts.next() {
                                if let Some(topic) = self.topic_map.get(room) {
                                    if let Err(e) = self.swarm.behaviour_mut().gossipsub.unsubscribe(topic) {
                                        errprintln(&format!("Failed to unsubscribe from topic: {}. Error: {:?}", room, e));
                                    } else {
                                        self.storage.remove_key(room);
                                        self.topic_map.remove(room);
                                    }
                                } else {
                                    errprintln(&format!("No such topic <{}> in your scope", room));
                                }
                            } else {
                                errprintln("Usage: unsub <room>");
                            }
                        }
                        Some("pub") => {
                            if let Some(room) = parts.next() {
                                if let Some(topic) = self.topic_map.get(room) {
                                    if let Ok(Some(line)) = stdin.next_line().await {
                                        if let Err(e) = self.swarm.behaviour_mut().gossipsub.publish(topic.clone(), line.as_bytes()) {
                                            errprintln(&format!("Publish error: {:?}", e));
                                        }
                                    }
                                } else {
                                    errprintln(&format!("No such topic <{}> in your scope", room));
                                }
                            } else {
                                errprintln("Usage: pub <room>");
                            }
                        }
                        Some("set-nickname") => {
                            if let (Some(id), Some(nickname)) = (parts.next(), parts.next()) {
                                self.accomplices_nicknames.insert(id.to_string(), nickname.to_string());
                            } else {
                                errprintln("Usage: set-nickname <id> <nickname>");
                            }
                        }
                        Some("req") => {
                            match parts.next() {
                                Some("file") => {
                                    if let (Some(peer), Some(file_name)) = (parts.next(), parts.next()) {
                                        let mut peer_id_str = peer;
                                        if let Some(acc) = self.get_accomplice_by_nickname(peer) {
                                            peer_id_str = acc;
                                        }
                                        if let Some(flag) = parts.next() {
                                            match flag {
                                                "-m" => {
                                                    match PeerId::from_str(peer_id_str) {
                                                        Ok(peer_id) => {
                                                            if let Ok(Some(line)) = stdin.next_line().await {
                                                                self.request_file(&peer_id, file_name, &line).await;
                                                            }
                                                        }
                                                        Err(_) => errprintln("Invalid peer ID"),
                                                    }
                                                }
                                                _ => errprintln(&format!("Unsupported flag: {}", flag)),
                                            }
                                        } else {
                                            match PeerId::from_str(peer_id_str) {
                                                Ok(peer_id) => {
                                                    self.request_file(&peer_id, file_name, "-/-").await;
                                                }
                                                Err(_) => errprintln("Invalid peer ID"),
                                            }
                                        }
                                    } else {
                                        errprintln("Usage: req file <peer_id> <file_name> [-m]");
                                    }
                                }
                                Some("menu") => {
                                    if let Some(peer) = parts.next() {
                                        let mut peer_id_str = peer;
                                        if let Some(acc) = self.get_accomplice_by_nickname(peer) {
                                            peer_id_str = acc;
                                        }
                                        match PeerId::from_str(peer_id_str) {
                                            Ok(peer_id) => self.request_menu(&peer_id).await,
                                            Err(_) => errprintln("Invalid peer ID"),
                                        }
                                    } else {
                                        errprintln("Usage: req menu <peer_id>");
                                    }
                                }
                                _ => errprintln("Usage: req file <peer_id> <file_name> [-m] / req menu <peer_id>"),
                            }
                        }
                        Some("res") => {
                            if let (Some(id), Some(file_path)) = (parts.next(), parts.next()) {
                                if let Ok(id) = id.parse::<u64>() {
                                    if self.requests_mailbox.contains_key(&id) {
                                        let path = Path::new(file_path);
                                        if path.exists() {
                                            let ((_, channel), request) =  self.requests_mailbox.remove(&id).unwrap();
                                            let response = FileOrMenuResponse {
                                                is_menu: false,
                                                menu: Vec::new(),
                                                file_name: request.file_name,
                                                file_data: read_file_as_bytes(file_path).unwrap_or_default(),
                                            };
                                            let _ = self.swarm.behaviour_mut().request_response.send_response(channel, response);
                                        } else {
                                            errprintln(&format!("No such file: {}", file_path));
                                        }
                                    } else {
                                        errprintln(&format!("No such request <{}> in your mailbox", id));
                                    }
                                } else {
                                    errprintln(&format!("Invalid id of request: {}", id));
                                }
                            } else {
                                errprintln("Usage: res file <request_id> <file_path>");
                            }
                        }
                        Some("mailbox") => {
                            self.list_incoming_requests();
                        }
                        Some("ls") => {
                            match parts.next() {
                                Some("accomplices") => self.list_accomplices(),
                                Some("rooms") => self.list_rooms(),
                                Some("requests") => self.list_outcoming_requests(),
                                _ => errprintln("Usage: ls accomplices/rooms/requests"),
                            }
                        }
                        Some("close") => {
                            match self.close() {
                                Ok(()) => {
                                    sysprintln("Client successfully saved&closed");
                                    break Ok(());
                                },
                                Err(e) => errprintln(&format!("Error while closing: {:?}", e)),
                            }
                        }
                        Some(other) => {
                            errprintln(&format!("Unknown command: {}", other));
                        }
                        None => {}
                    }
                }
                event = self.swarm.select_next_some() => match event {
                    SwarmEvent::Behaviour(MyBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                        for (peer_id, _) in list {
                            if !self.accomplices.contains(&peer_id.to_string()) {
                                println!("/global/ There's a new peer <{}> on the horizon", peer_id);
                                self.accomplices.insert(peer_id.to_string());
                                self.swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                            }
                        }
                    }
                    SwarmEvent::Behaviour(MyBehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
                        for (peer_id, _) in list {
                            println!("/global/ Peer <{}> shows no signs of life", peer_id);
                            self.swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer_id);
                        }
                    }
                    SwarmEvent::Behaviour(MyBehaviourEvent::Gossipsub(gossipsub::Event::Message {
                        propagation_source: peer_id,
                        message_id: _,
                        message,
                    })) => {
                        let topic = &message.topic;
                        let message_data = String::from_utf8_lossy(&message.data);
                        println!("/{}/ -> {}:", self.storage.get_room(&topic.to_string()).unwrap(), self.accomplices_nicknames.get(&peer_id.to_string()).unwrap_or(&peer_id.to_string()));
                        println!("\t{}", message_data);
                    },
                    SwarmEvent::NewListenAddr { address, .. } => {
                        sysprintln(&format!("Local node is listening on {}", address));
                    },
                    SwarmEvent::Behaviour(MyBehaviourEvent::RequestResponse(libp2p_request_response::Event::Message { peer, message })) => {
                        match message {
                            libp2p_request_response::Message::Request { request_id: _, request, channel } => {
                                if request.is_menu {
                                    let response = FileOrMenuResponse {
                                        is_menu: true,
                                        menu: self.get_menu(),
                                        file_name: "".to_string(),
                                        file_data: Vec::new(),
                                    };
                                    let _ = self.swarm.behaviour_mut().request_response.send_response(channel, response);
                                } else if !self.get_menu().contains(&request.file_name) {
                                    let mut hasher = DefaultHasher::new();
                                    let all_in_one = peer.to_string() + &request.message + &request.file_name;
                                    all_in_one.hash(&mut hasher);
                                    sysprintln(&format!("Request received from {}", self.accomplices_nicknames.get(&peer.to_string()).unwrap_or(&peer.to_string())));
                                    self.requests_mailbox.insert(hasher.finish(), ((peer.to_string(), channel), request));
                                } else {
                                    let full_path = self.pub_dir_path.clone() + "/" + &request.file_name;
                                    let response = FileOrMenuResponse {
                                        is_menu: false,
                                        menu: Vec::new(),
                                        file_name: request.file_name,
                                        file_data: read_file_as_bytes(&full_path).unwrap_or_default(),
                                    };
                                    let _ = self.swarm.behaviour_mut().request_response.send_response(channel, response);
                                }
                            },
                            libp2p_request_response::Message::Response { request_id: _, response } => {
                                sysprintln(&format!("Response received from {}", self.accomplices_nicknames.get(&peer.to_string()).unwrap_or(&peer.to_string())));
                                if !response.is_menu {
                                    let file_path = "./drain".to_string() + "/" + &response.file_name;
                                    self.pending_requests.retain(|(requester, request)| request.file_name != response.file_name || requester != &peer.to_string());
                                    match save_bytes_to_file(&file_path, response.file_data) {
                                        Ok(()) => {
                                            println!("\t |Data saved successfully to {}", file_path);
                                            println!("\t ---");
                                        }
                                        Err(e) => errprintln(&format!("Error saving data to {}: {}", file_path, e)),
                                    }
                                } else {
                                    println!("\t |Menu:");
                                    for pos in response.menu {
                                        println!("\t |{}", pos);
                                    }
                                    println!("\t ---");
                                }
                            },
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
