#!/usr/bin/env python3
"""Decode and inspect a labelled qualification take with a full FFmpeg/ffprobe toolchain."""
import json,pathlib,subprocess,sys
root=pathlib.Path(sys.argv[1]);report={};durations=[]
for name in ['screen','camera','recording']:
 file=root/(name+'.mp4')
 data=json.loads(subprocess.check_output(['ffprobe','-v','error','-count_frames','-show_entries','stream=codec_type,width,height,duration,nb_frames,nb_read_frames,start_time','-of','json',str(file)]))
 subprocess.run(['ffmpeg','-v','error','-xerror','-i',str(file),'-f','null','-'],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
 streams=data['streams'];video=next(s for s in streams if s['codec_type']=='video');assert int(video['nb_read_frames'])>0
 durations.append(float(video['duration']));audio=[s for s in streams if s['codec_type']=='audio']
 if name=='camera':assert not audio;assert(video['width'],video['height'])==(1280,720)
 else:
  assert audio,'Qualification tone must be present'
  assert abs(float(audio[0]['duration'])-float(video['duration']))<0.12,'Audio/video duration drift exceeds 120 ms'
 report[name]={'decoded':True,'streams':streams}
assert max(durations)-min(durations)<0.05,'Video streams do not share a timeline'
counts=[next(s for s in report[n]['streams']if s['codec_type']=='video')['nb_read_frames']for n in report];assert len(set(counts))==1
print(json.dumps({'passed':True,'files':report},indent=2))
