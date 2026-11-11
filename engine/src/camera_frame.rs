//! Bounded self-describing camera frames. Native aspect ratio is retained; a
//! square preview is a crop of the content, never a crop of padded black bars.
use std::io::{self, BufRead, Read};
#[derive(Debug)]
pub struct Frame { pub width: usize, pub height: usize, pub bgra: Vec<u8>, pub sequence: u64 }
pub fn read_pam(input: &mut impl BufRead) -> io::Result<Frame> {
    let mut header = Vec::new();
    let mut width = None; let mut height = None; let mut depth = None; let mut maximum = None;
    loop {
        let mut line = Vec::new();
        let count = input.by_ref().take((4097-header.len()) as u64).read_until(b'\n', &mut line)?;
        if count == 0 { return Err(io::Error::new(io::ErrorKind::UnexpectedEof,"Camera frame header ended")); }
        header.extend_from_slice(&line);
        if header.len()>4096 || !line.ends_with(b"\n") { return Err(io::Error::other("Camera frame header exceeds limit")); }
        let text=std::str::from_utf8(&line).map_err(|_|io::Error::other("Invalid camera frame header"))?.trim();
        if header.len()==line.len() { if text!="P7" {return Err(io::Error::other("Expected PAM camera frame"));} continue; }
        if text=="ENDHDR" { break; }
        if text.starts_with('#') || text.is_empty() {continue;}
        if let Some((key,value))=text.split_once(' ') {
            let parsed=value.trim().parse::<usize>().ok();
            match key {"WIDTH"=>width=parsed,"HEIGHT"=>height=parsed,"DEPTH"=>depth=parsed,"MAXVAL"=>maximum=parsed,_=>{}}
        }
    }
    let (w,h)=(width.ok_or_else(||io::Error::other("Camera width missing"))?,height.ok_or_else(||io::Error::other("Camera height missing"))?);
    if w<2 || h<2 || w>3840 || h>3840 || w*h>3840*2160 || depth!=Some(4) || maximum!=Some(255) {return Err(io::Error::other("Unsupported camera frame dimensions or pixel format"));}
    let mut bgra=vec![0;w*h*4];input.read_exact(&mut bgra)?;
    for pixel in bgra.chunks_exact_mut(4) {pixel.swap(0,2);}
    Ok(Frame{width:w,height:h,bgra,sequence:0})
}
#[cfg(test)]mod tests{
 use super::*;use std::io::Cursor;
 fn frame(w:usize,h:usize)->Vec<u8>{let mut v=format!("P7\nWIDTH {w}\nHEIGHT {h}\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n").into_bytes();for _ in 0..w*h{v.extend_from_slice(&[10,20,30,255]);}v}
 #[test]fn portrait_and_landscape_preserve_full_content(){for(w,h)in[(4,6),(8,4)]{let f=read_pam(&mut Cursor::new(frame(w,h))).unwrap();assert_eq!((f.width,f.height),(w,h));assert_eq!(f.bgra.len(),w*h*4);assert!(f.bgra.chunks_exact(4).all(|p|p==[30,20,10,255]));}}
 #[test]fn concatenated_frames_have_exact_boundaries(){let mut v=frame(4,6);v.extend(frame(8,4));let mut c=Cursor::new(v);assert_eq!(read_pam(&mut c).unwrap().width,4);assert_eq!(read_pam(&mut c).unwrap().width,8);assert_eq!(read_pam(&mut c).unwrap_err().kind(),io::ErrorKind::UnexpectedEof);}
 #[test]fn truncated_or_unbounded_input_is_rejected(){let mut v=frame(4,6);v.pop();assert!(read_pam(&mut Cursor::new(v)).is_err());assert!(read_pam(&mut Cursor::new(format!("P7\n{}","x".repeat(5000)))).is_err());assert!(read_pam(&mut Cursor::new(frame(3841,2))).is_err());}
}
