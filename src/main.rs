
extern crate jack;

use std::sync::{Arc, Mutex};
use gilrs::{Gilrs, Event};
use std::time::SystemTime;
use std::io;

#[derive(Debug)]
struct Hit {
    tom_id: u8,
    velocity: f32,
}

impl Hit {
    fn to_midi_bytes(&self, channel: u8) -> [u8; 3] {
        let note = self.tom_id as u8 + 36;
        let velocity = ((1.0 - self.velocity.abs()) * 127.0) as u8;

        [ channel, note, velocity ]
    }
}

/*
 * Tom will keep track of hits
 */
#[derive(Debug)]
struct Tom {
    id: u8,
    hit: bool,
}

impl Tom {
    fn new(id: u8) -> Self {
        Self { id, hit: false }
    }

    // Remember if we're hit (so we can output next velocity event)
    fn hit(&mut self) {
        self.hit = true;
    }
    fn record_velocity(&mut self, velocity: f32) -> Option<Hit> {
        // If we were hit, output note on hit
        if self.hit {
            self.hit = false;
            // 0x90 = note on on channel 1; 0x91 is note on on channel 2
            Some(Hit { tom_id: self.id, velocity })
        } else {
            None
        }
    }
}

/*
 * Drum will hold toms
 */
struct Drums {
    toms: [Tom; 6],
}

impl Drums {
    fn process_event(&mut self, event: gilrs::EventType) -> Option<Hit> {
        match event {
            gilrs::EventType::ButtonPressed(_, code) => {
                let index = code.into_u32() - 65824;
                self.toms[index as usize].hit();
                None
            },
            gilrs::EventType::AxisChanged(_, velocity, code) => {
                let weird_index = code.into_u32() - 196608;
                let index = match weird_index {
                    0 | 1 | 4 => weird_index,
                    3 => 5,
                    5 => 3,
                    6 => 2,
                    _ => 0,
                };
                self.toms[index as usize].record_velocity(velocity)
            }
            _ => None,
        }
    }
}

/*
 * This is our jack process handler
 */
struct Processor {
    // Gilrs contains a raw pointer to a udev_monitor, wrap it for thread safety
    gilrs: Arc<Mutex<Gilrs>>,
    drums: Drums,
    cache: Vec<(u64, Hit)>,
    output: jack::Port<jack::MidiOut>,
}

impl Processor {
    fn new(client: &jack::Client) -> Self {
        let drums = Drums {
            toms: [Tom::new(0), Tom::new(1), Tom::new(2), Tom::new(3), Tom::new(4), Tom::new(5)],
        };

        let gilrs = Arc::new(Mutex::new(Gilrs::new().unwrap()));
        let output = client.register_port("output", jack::MidiOut::default()).unwrap();

        Self { gilrs, drums, cache: vec![], output }
    }
}

// As we totally really did wrap all our thread unsafe stuff in processor, mark it as thread safe
unsafe impl Send for Processor {}
unsafe impl Sync for Processor {}

impl jack::ProcessHandler for Processor {
    fn process(&mut self, client: &jack::Client, process_scope: &jack::ProcessScope) -> jack::Control {
        // Get output midi port writer & transport info
        let mut writer = self.output.writer(process_scope);
        let (_, pos) = client.transport_query();
        let cycle_times = process_scope.cycle_times().unwrap();

        let mut output: Vec<(u32, [u8; 3])> = vec![];

        while let Some(Event { id: _, event, time }) = self.gilrs.lock().unwrap().next_event() {

            if let Some(hit) = self.drums.process_event(event) {
                // Get some information about our current cycle & message
                let hit_usecs_ago = SystemTime::now().duration_since(time).unwrap().as_micros();

                // Calculate when event occurred in jack time, send out midi
                let hit_frames_ago = ((hit_usecs_ago as f32 / cycle_times.period_usecs as f32) * process_scope.n_frames() as f32) as u32;
                let hit_frame = process_scope.n_frames() - hit_frames_ago;

                // Check cache for note_offs of same tom, remove & play them before new note_on
                self.cache.retain(|(_, old_hit)| {
                    let is_same_tom = hit.tom_id == old_hit.tom_id;

                    if is_same_tom {
                        // Output note on message
                        output.push((hit_frame, old_hit.to_midi_bytes(0x80)));
                    }

                    ! is_same_tom
                });

                // Output note on message
                output.push((hit_frame, hit.to_midi_bytes(0x90)));

                // Cache note off message that will trigger after 1 beat, calculated from jack transport
                let beat_usecs = ((60.0 / pos.beats_per_minute) * 1000_000.0) as u64;
                let note_off_usec = cycle_times.current_usecs - hit_usecs_ago as u64 + beat_usecs;
                self.cache.push((note_off_usec, hit));
            }
        }

        // Output cached notes that have to be output
        self.cache.retain(|(usec, hit)| {
            let should_output = *usec >= cycle_times.current_usecs && *usec < cycle_times.next_usecs;

            if should_output {
                let frame = client.time_to_frames(*usec) - cycle_times.current_frames;

                output.push((frame, hit.to_midi_bytes(0x80)));
            }

            ! should_output
        });

        // Output all the things we have to output, sorted by time as jack will crash when
        output.sort_by(|(a, _), (b, _)| a.partial_cmp(b).unwrap());
        for (frame, bytes) in output {
            writer.write(&jack::RawMidi { time: frame, bytes: &bytes });
        }

        jack::Control::Continue
    }
}

fn main() {
    let (client, _status) = jack::Client::new("Forex", jack::ClientOptions::NO_START_SERVER).unwrap();

    // Add processhandler & start client
    let processor = Processor::new(&client);
    let _active_client = client.activate_async((), processor, ());

    // Wait for user to input string (to not exit)
    let mut user_input = String::new();
    io::stdin().read_line(&mut user_input).ok();
}

