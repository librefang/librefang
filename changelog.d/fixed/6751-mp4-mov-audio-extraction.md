Transcription of `.mp4` and `.mov` recordings works again instead of silently returning nothing.
  The audio track was extracted by piping the container to ffmpeg, but these formats keep their index at the end of the file and demuxing it requires seeking backwards, which a pipe cannot do — so the extraction produced a valid container carrying no audio at all, and whatever the transcription provider made of a soundless file was what the operator saw.
  Because ffmpeg reports success in this case, neither existing check noticed.
  The input is now staged to a scratch file so it can be seeked, and a stream that arrives without audio is rejected outright rather than uploaded.
  `.mkv` and `.avi` were never affected, being streamable formats.
  (#6751) (@houko)
