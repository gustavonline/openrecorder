//! OpenRecorder: fixed-capacity frame delivery; drop incoming frames under pressure.
//! Capture callbacks must never block while the app is stopping.
use std::sync::mpsc::{self,Receiver,SendError,TrySendError};
#[derive(Debug)]
pub struct Sender<T>(mpsc::SyncSender<T>);
impl<T> Clone for Sender<T>{fn clone(&self)->Self{Self(self.0.clone())}}
impl<T> Sender<T>{pub fn send(&self,item:T)->Result<(),SendError<T>>{match self.0.try_send(item){Ok(())|Err(TrySendError::Full(_))=>Ok(()),Err(TrySendError::Disconnected(item))=>Err(SendError(item))}}}
pub fn channel<T>()->(Sender<T>,Receiver<T>){let(tx,rx)=mpsc::sync_channel(2);(Sender(tx),rx)}
#[cfg(test)]mod tests{use super::*;#[test]fn pressure_never_accumulates_more_than_two_frames(){let(tx,rx)=channel();for i in 0..100_000{tx.send(i).unwrap();}assert_eq!(rx.try_iter().collect::<Vec<_>>(),vec![0,1]);}#[test]fn disconnected_receiver_is_reported(){let(tx,rx)=channel();drop(rx);assert!(tx.send(1).is_err());}}
