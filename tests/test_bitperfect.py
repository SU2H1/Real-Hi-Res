"""The decisions and texts of bit-perfect playback (realhires/bitperfect.py, 2.1),
which the Rust core gives too (rust/crates/core/tests/bitperfect_vectors.rs)."""

from realhires import bitperfect as bp
from realhires.bitperfect import F32, I16, I24, I32, Mode, Song
from realhires.settings import Settings
from realhires.store import settings_key


def modes(formats, rates, channels=2):
    return [Mode(f, r, r, channels) for f in formats for r in rates]


def test_the_song_gets_its_own_rate_in_the_best_format():
    cd, hi, flt = Song(44100, 2, 16), Song(96000, 2, 24), Song(48000, 2, 32, is_float=True)
    full = modes((I16, I24, I32, F32), (44100, 48000, 96000))
    assert (bp.plan(cd, full).format, bp.plan(hi, full).format, bp.plan(flt, full).format) == (I16, I32, F32)
    assert bp.plan(cd, full).reasons == [] and bp.plan(cd, full).rate == 44100
    assert bp.plan(cd, modes((I32, F32), (44100,))).format == I32
    assert bp.plan(hi, modes((I24, F32), (96000,))).format == I24
    assert bp.plan(hi, modes((F32,), (96000,))).format == F32 and not bp.plan(hi, modes((F32,), (96000,))).reasons
    assert bp.plan(cd, []).format == I16 and bp.plan(cd, []).rate == 44100  # [] = anything
    # a range of rates, as CoreAudio and cpal report them
    assert bp.plan(Song(88200, 2, 24), [Mode(F32, 44100, 192000, 0)]).rate == 88200


def test_reasons_are_full_sentences():
    hi = Song(192000, 2, 24)
    p = bp.plan(hi, modes((I16,), (44100, 48000, 96000)))
    assert p.rate == 96000 and p.format == I16 and p.resampled
    assert p.reasons == ["The device cannot play 192 kHz, so the song is resampled to 96 kHz.",
                         "The device has no 24-bit format, so the samples are converted."]
    assert bp.status(hi, p, bp.WASAPI) == "No: " + " ".join(p.reasons)
    six = bp.plan(Song(48000, 6, 24), modes((I32,), (48000,)))
    assert six.reasons == ["This 6-channel song is mixed down to stereo."]
    s32 = bp.plan(Song(96000, 2, 32), modes((I32,), (96000,)))
    assert s32.reasons == ["The 32-bit samples are played with 24-bit precision."]
    busy = bp.plan(Song(44100, 2, 16), modes((F32,), (44100,)), failure=bp.BUSY)
    assert bp.status(Song(44100, 2, 16), busy) == ("No: The device is in use by another program, so the sound is "
                                                   "mixed by the system.")
    assert bp.failure_reason("ALSA: No such device.") == ("The device could not be opened for this app alone "
                                                         "(ALSA: No such device), so the sound is mixed by the system.")


def test_status_texts():
    hi = Song(96000, 2, 24)
    p = bp.plan(hi, modes((I32,), (96000,)))
    assert bp.status(hi, p, bp.WASAPI) == "Yes: 24-bit, 96 kHz, exclusive mode (WASAPI)"
    cd = Song(44100, 2, 16)
    assert bp.status(cd, bp.plan(cd, modes((I16,), (44100,))), bp.ALSA, "hw:0") == \
        "Yes: 16-bit, 44.1 kHz, ALSA hardware device hw:0"
    assert bp.status(hi, bp.plan(hi, [Mode(F32, 44100, 96000, 0)]), bp.COREAUDIO) == \
        "Yes: 24-bit, 96 kHz, the device runs at 96 kHz (CoreAudio)"
    lossy = Song(44100, 2, 0, is_lossy=True)
    assert bp.status(lossy, bp.plan(lossy, modes((F32,), (44100,))), bp.DIRECT, "Speakers") == \
        "Yes (decoded lossy audio): 32-bit float, 44.1 kHz, Speakers"
    assert bp.status(lossy, bp.plan(lossy, modes((I16,), (48000,)))) == (
        "No: The device cannot play 44.1 kHz, so the song is resampled to 48 kHz. The device has no 32-bit float "
        "format, so the samples are converted. The song is decoded from a lossy format, so it is played as decoded.")
    assert bp.status(None, None, enabled=False) == "Off" and bp.status(None, None) == "On"
    assert bp.output_label(96000, I32, 24) == "96 kHz, 24-bit integer in 32-bit, stereo"
    assert bp.output_label(44100, I16, 16) == "44.1 kHz, 16-bit integer, stereo"
    assert bp.output_label(48000, F32, 0, 8) == "48 kHz, 32-bit float, 8 channels"


def test_rates_when_the_song_rate_is_missing():
    assert bp.pick_rate(44100, [48000, 96000]) == 48000
    assert bp.pick_rate(44100, [48000, 88200, 96000]) == 88200      # a whole multiple
    assert bp.pick_rate(176400, [44100, 48000, 88200, 96000]) == 88200
    assert bp.pick_rate(192000, [44100, 48000, 96000]) == 96000
    assert bp.pick_rate(22050, []) == 22050


def test_exact_integers():
    assert bp.int_sample(0.5, I16) == 16384 and bp.int_sample(1.0, I16) == 32767 and bp.int_sample(-1.0, I16) == -32768
    assert bp.int_sample(8388607 / 8388608, I24) == 8388607 and bp.int_sample(-2.0, I32) == -2 ** 31
    assert bp.int_sample(float("nan"), I32) == 0 and bp.int_sample(1.5 / 32768, I16) == 2  # halves to even


def test_alsa_device_names():
    cards = bp.alsa_cards(" 0 [PCH            ]: HDA-Intel - HDA Intel PCH\n"
                          "                      HDA Intel PCH at 0xf7f10000 irq 32\n"
                          " 1 [DAC            ]: USB-Audio - USB Audio DAC\n"
                          "                      Manufacturer USB Audio DAC at usb-0000:00:14.0-1, high speed\n")
    assert cards == [(0, "PCH", "HDA Intel PCH"), (1, "DAC", "USB Audio DAC")]
    assert bp.alsa_device("hw:CARD=DAC,DEV=0", cards) == "hw:1"
    assert bp.alsa_device("plughw:0,3", cards) == "hw:0,3"
    assert bp.alsa_device("USB Audio DAC Analog Stereo", cards) == "hw:1"
    assert bp.alsa_device("", cards) == "hw:0" and bp.alsa_device("Something else", []) == "hw:0"


def test_the_setting_is_not_a_detection_setting():
    info = {f["name"]: f for f in Settings.field_info()}
    assert info["bit_perfect"]["group"] == "Playback" and info["bit_perfect"]["default"] is False
    assert info["bit_perfect"]["label"] == "Bit-perfect playback"
    names = [f["name"] for f in Settings.field_info()]
    assert names.index("bit_perfect") == names.index("gapless") + 1
    assert settings_key(Settings(bit_perfect=True)) == settings_key(Settings())
