import React from "react";

function fmt_uptime( value ) {
    const days = Math.floor( value / (3600 * 24) );
    value -= days * (3600 * 24);
    const hours = Math.floor( value / 3600 );
    value -= hours * 3600;
    const minutes = Math.floor( value / 60 );
    value -= minutes * 60;
    const seconds = value;

    let output = seconds + "s";
    if( minutes > 0 ) {
        output = minutes + "m " + output;
    }

    if( hours > 0 ) {
        output = hours + "h " + output;
    }

    if( days > 0 ) {
        output = days + "d " + output;
    }

    return output;
}

function fmt_uptime_timeval( timeval ) {
    let secs = timeval.secs;
    const days = Math.floor( secs / (3600 * 24) );
    secs -= days * (3600 * 24);
    const hours = Math.floor( secs / 3600 );
    secs -= hours * 3600;
    const minutes = Math.floor( secs / 60 );
    secs -= minutes * 60;

    let nsecs = timeval.fract_nsecs;
    const usecs = Math.floor( nsecs / 1000 );
    nsecs -= usecs * 1000;
    const msecs = Math.floor( usecs / 1000 );
    nsecs -= msecs * 1000;

    let output = "";

    if( secs === 0 ) {
        if( msecs === 0 ) {
            if( usecs === 0 ) {
                if( nsecs === 0 ) {
                    output = "0" + output;
                } else {
                    output = nsecs + "ns" + output;
                }
            } else {
                output = usecs + "us" + output;
            }
        } else {
            output = msecs + "ms" + output;
        }
    }

    if( secs > 0 ) {
        output = secs + "s " + output;
    }

    if( minutes > 0 ) {
        output = minutes + "m " + output;
    }

    if( hours > 0 ) {
        output = hours + "h " + output;
    }

    if( days > 0 ) {
        output = days + "d " + output;
    }

    return output.trim();
}

const T = 1000 * 1000 * 1000 * 1000;
const G = 1000 * 1000 * 1000;
const M = 1000 * 1000;
const K = 1000;

function fmt_size_impl( value, mul, suffix, force_fract ) {
    const whole = Math.floor( value / mul );
    const fract = Math.floor( (value - whole * mul) / (mul / 1000) );

    if( force_fract || fract !== 0 ) {
        let fract_s = fract + "";
        while( fract_s.length < 3 ) {
            fract_s = "0" + fract_s;
        }

        return whole + "." + fract_s + " " + suffix;
    } else {
        return whole + " " + suffix;
    }
}

function fmt_size( value, force_fract ) {
    if( force_fract === undefined ) {
        force_fract = true;
    }

    const abs_value = Math.abs( value );
    if( abs_value >= T ) {
        return fmt_size_impl( value, T, "T", force_fract );
    }

    if( abs_value >= G ) {
        return fmt_size_impl( value, G, "G", force_fract );
    }

    if( abs_value >= M ) {
        return fmt_size_impl( value, M, "M", force_fract );
    }

    if( abs_value >= K ) {
        return fmt_size_impl( value, K, "K", force_fract );
    }

    return value;
}

function fmt_full_size( value ) {
    let output = "";
    let abs_value = Math.abs( value );
    let is_first = true;

    const process = function( mul, suffix ) {
        if( abs_value < mul ) {
            return;
        }

        const whole = Math.floor( abs_value / mul );
        abs_value -= whole * mul;

        let whole_s = whole + "";
        if( !is_first ) {
            output += " ";
            while( whole_s.length < 3 ) {
                whole_s = "0" + whole_s;
            }
        }

        is_first = false;
        output += whole_s + suffix;
    }

    process( T, "T" );
    process( G, "G" );
    process( M, "M" );
    process( K, "K" );
    process( 1, "" );

    return output;
}

function fmt_date( date ) {
    const y = date.getUTCFullYear();

    if( isNaN( y ) ) {
        return null;
    }

    let m = date.getUTCMonth() + 1;
    if (m < 10) {
        m = "0" + m;
    } else {
        m = "" + m;
    }

    let d = date.getUTCDate();
    if (d < 10) {
        d = "0" + d;
    } else {
        d = "" + d;
    }

    let h = date.getUTCHours();
    if (h < 10) {
        h = "0" + h;
    } else {
        h = "" + h;
    }

    let min = date.getUTCMinutes();
    if (min < 10) {
        min = "0" + min;
    } else {
        min = "" + min;
    }

    let s = date.getUTCSeconds();
    if (s < 10) {
        s = "0" + s;
    } else {
        s = "" + s;
    }

    let ms = date.getUTCMilliseconds();
    if( ms !== 0 ) {
        ms = "" + ms;
        while( ms.length < 3 ) {
            ms = "0" + ms;
        }
        ms = "." + ms;
    } else {
        ms = "";
    }

    return y + "-" + m + "-" + d + " " + h + ":" + min + ":" + s + ms;
}

function fmt_date_timeval( timestamp ) {
    let output = fmt_date( new Date( timestamp.secs * 1000 ) );
    let s = ((timestamp.fract_nsecs / 1000000) | 0) + "";
    while (s.length < 3) {
        s = "0" + s;
    }
    return output + "." + s
}

function fmt_date_unix( timestamp ) {
    return fmt_date( new Date( timestamp * 1000 ) );
}

function fmt_date_unix_ms( timestamp ) {
    return fmt_date( new Date( parseInt( timestamp, 10 ) ) );
}

function fmt_hex16( value ) {
    if( value === undefined || value === null ) {
        return value;
    }

    let output = value.toString( 16 );
    while( output.length < 4 ) {
        output = "0" + output;
    }

    return output;
}

function fmt_duration_for_display( value ) {
    value = value.replace( " ", "" ).toLowerCase();
    if( value.match( /^(\d+)$/ ) ) {
        value = value + "s";
    }

    return value;
}

function update_query( props, obj ) {
    let q = new  URLSearchParams( props.location.search );
    const keys = Object.keys( obj );
    for( let i = 0; i < keys.length; ++i ) {
        const key = keys[ i ];
        const value = obj[ key ];
        if( value === undefined || value === null ) {
            q.delete( key );
        } else {
            q.set( key, value );
        }
    }

    const location = {...props.location, search: "?" + q.toString() };
    props.history.replace( location );

    return location;
}

function create_query( object ) {
    const q = new URLSearchParams();
    const keys = Object.keys( object );
    for( let i = 0; i < keys.length; ++i ) {
        const key = keys[ i ];
        const value = object[ key ];
        if( value === undefined || value === null ) {
            continue;
        }

        q.set( key, value );
    }

    return q;
}

function extract_query( query ) {
    const output = {};
    const q = new URLSearchParams( query );
    for( let entry of q.entries() ) {
        output[ entry[ 0 ] ] = entry[ 1 ];
    }

    return output;
}

function format_frame( index, frame ) {
    const key = "frame_" + index;
    if( frame.function || frame.raw_function ) {
        let f = frame.function || frame.raw_function;
        if( frame.source && frame.line ) {
            f += " [" + frame.source.match( /[^\/]*$/ )[ 0 ] + ":" + frame.line + "]";
        }
        return <div key={key}>#{index} [{frame.library}] {f}</div>;
    }
    return <div key={key}>#{index} [{frame.library}] {"0x" + frame.address_s}</div>;
}

function def( value, default_value ) {
    if( value !== undefined && value !== null ) {
        return value;
    } else {
        return default_value;
    }
}

export { fmt_uptime, fmt_uptime_timeval, fmt_size, fmt_full_size, fmt_date, fmt_date_unix, fmt_date_unix_ms, fmt_date_timeval, fmt_hex16, fmt_duration_for_display, update_query, create_query, extract_query, format_frame, def }
